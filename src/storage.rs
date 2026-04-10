use crate::models::{
    AlertEvent, AlertLevel, AppSettings, FoldsModelRecord, Instrument, Observation,
    SavedInference, FOLDS_STATUS_PENDING,
};
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{Connection, params};
use std::path::Path;

pub struct Storage {
    conn: Connection,
}

impl Storage {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open SQLite database at {}", path.display()))?;
        let mut storage = Self { conn };
        storage.init()?;
        Ok(storage)
    }

    /// Open a temporary in-memory database. Used as a fallback when the real
    /// database file cannot be opened, so the app struct remains valid while
    /// an error message is shown to the user.
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .context("failed to open in-memory fallback database")?;
        let mut storage = Self { conn };
        storage.init()?;
        Ok(storage)
    }

    fn init(&mut self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;

            CREATE TABLE IF NOT EXISTS observations (
                instrument TEXT NOT NULL,
                date TEXT NOT NULL,
                close REAL NOT NULL,
                source TEXT NOT NULL,
                PRIMARY KEY (instrument, date, source)
            );

            CREATE INDEX IF NOT EXISTS idx_obs_instrument ON observations(instrument, date);

            CREATE TABLE IF NOT EXISTS app_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS alert_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp_utc TEXT NOT NULL,
                instrument TEXT NOT NULL,
                level TEXT NOT NULL,
                note TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS knowledge_chunks (
                id    INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                tags  TEXT NOT NULL,
                body  TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS ai_inferences (
                id                  INTEGER PRIMARY KEY AUTOINCREMENT,
                created_at          TEXT NOT NULL,
                provider            TEXT NOT NULL,
                model               TEXT NOT NULL,
                system_prompt       TEXT NOT NULL,
                user_message        TEXT NOT NULL,
                response            TEXT NOT NULL,
                vix_close           REAL,
                vix_level           TEXT,
                hypothesis_question TEXT,
                hypothesis_outcomes TEXT,
                hypothesis_context  TEXT,
                overlay_instruments TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_ai_inferences_created_at
                ON ai_inferences(created_at);

            CREATE TABLE IF NOT EXISTS folds_models (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                model_id        TEXT NOT NULL UNIQUE,
                status          TEXT NOT NULL,
                created_at      TEXT NOT NULL,
                completed_at    TEXT,
                last_polled_at  TEXT,
                question        TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_folds_models_status
                ON folds_models(status);
            "#,
        )?;

        // Additive migration: an older version of this app shipped
        // `ai_inferences` without the four hypothesis/overlay columns. The
        // CREATE TABLE above is `IF NOT EXISTS`, so for upgraded databases
        // we have to ALTER. SQLite has no `ADD COLUMN IF NOT EXISTS`, so
        // we query the current column set first and only ALTER what's
        // missing. New databases hit this code with all four columns
        // already present and the loop is a no-op.
        let existing_cols = self.column_set("ai_inferences")?;
        let migrations: &[(&str, &str)] = &[
            ("hypothesis_question",
             "ALTER TABLE ai_inferences ADD COLUMN hypothesis_question TEXT"),
            ("hypothesis_outcomes",
             "ALTER TABLE ai_inferences ADD COLUMN hypothesis_outcomes TEXT"),
            ("hypothesis_context",
             "ALTER TABLE ai_inferences ADD COLUMN hypothesis_context TEXT"),
            ("overlay_instruments",
             "ALTER TABLE ai_inferences ADD COLUMN overlay_instruments TEXT"),
        ];
        for (col, sql) in migrations {
            if !existing_cols.contains(*col) {
                self.conn.execute(sql, [])?;
            }
        }

        // Additive migration for folds_models — add columns for linking
        // models to AI analyses and storing the full SDK response.
        let folds_cols = self.column_set("folds_models")?;
        let folds_migrations: &[(&str, &str)] = &[
            ("inference_id",
             "ALTER TABLE folds_models ADD COLUMN inference_id INTEGER REFERENCES ai_inferences(id)"),
            ("response_json",
             "ALTER TABLE folds_models ADD COLUMN response_json TEXT"),
            ("outcomes",
             "ALTER TABLE folds_models ADD COLUMN outcomes TEXT"),
            ("short_summary",
             "ALTER TABLE folds_models ADD COLUMN short_summary TEXT"),
        ];
        for (col, sql) in folds_migrations {
            if !folds_cols.contains(*col) {
                self.conn.execute(sql, [])?;
            }
        }

        Ok(())
    }

    /// Return the set of column names declared on a table. Used by
    /// additive migrations to detect which columns are already present.
    fn column_set(&self, table: &str) -> Result<std::collections::HashSet<String>> {
        let mut stmt = self
            .conn
            .prepare(&format!("PRAGMA table_info({table})"))?;
        let cols: std::collections::HashSet<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(cols)
    }

    pub fn load_settings(&self) -> Result<AppSettings> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM app_settings WHERE key = 'settings'")?;
        let mut rows = stmt.query([])?;

        if let Some(row) = rows.next()? {
            let value: String = row.get(0)?;
            if value.len() > 65_536 {
                return Err(anyhow!(
                    "settings blob is suspiciously large ({} bytes); refusing to deserialise",
                    value.len()
                ));
            }
            Ok(serde_json::from_str(&value).context("failed to deserialize app settings")?)
        } else {
            Ok(AppSettings::default())
        }
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<()> {
        let value = serde_json::to_string_pretty(settings)?;
        self.conn.execute(
            r#"
            INSERT INTO app_settings (key, value)
            VALUES ('settings', ?1)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value
            "#,
            params![value],
        )?;
        Ok(())
    }

    pub fn load_observations(&self, instrument: Instrument) -> Result<Vec<Observation>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT date, close, source
            FROM observations
            WHERE instrument = ?1
            ORDER BY date ASC
            "#,
        )?;

        let rows = stmt.query_map(params![instrument.storage_key()], |row| {
            let date_str: String = row.get(0)?;
            let close: f64 = row.get(1)?;
            let source: String = row.get(2)?;
            Ok((date_str, close, source))
        })?;

        let mut observations = Vec::new();
        for row in rows {
            let (date_str, close, source) = row?;
            let date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")
                .with_context(|| format!("invalid date stored in db: {date_str}"))?;
            observations.push(Observation {
                instrument,
                date,
                close,
                source: intern_source(&source),
            });
        }

        Ok(observations)
    }

    /// Returns the most recent observation date for `instrument` whose `source`
    /// starts with `source_prefix`. Used for daily-cache checks where the
    /// caller knows the provider (e.g. "Alpha Vantage") but not the per-symbol
    /// suffix ("Alpha Vantage GOLD"). Pass the full source string to match
    /// exactly (e.g. "FRED VIXCLS").
    pub fn last_observation_date_for_provider(
        &self,
        instrument: Instrument,
        source_prefix: &str,
    ) -> Result<Option<NaiveDate>> {
        let pattern = format!("{source_prefix}%");
        let mut stmt = self.conn.prepare(
            "SELECT MAX(date) FROM observations \
             WHERE instrument = ?1 AND source LIKE ?2",
        )?;
        let date_str: Option<String> = stmt.query_row(
            params![instrument.storage_key(), pattern],
            |row| row.get(0),
        )?;

        match date_str {
            Some(s) => {
                let date = NaiveDate::parse_from_str(&s, "%Y-%m-%d")
                    .with_context(|| format!("invalid date stored in db: {s}"))?;
                Ok(Some(date))
            }
            None => Ok(None),
        }
    }

    pub fn replace_observations(
        &mut self,
        instrument: Instrument,
        observations: &[Observation],
    ) -> Result<usize> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM observations WHERE instrument = ?1",
            params![instrument.storage_key()],
        )?;

        let mut stmt = tx.prepare(
            r#"
            INSERT INTO observations (instrument, date, close, source)
            VALUES (?1, ?2, ?3, ?4)
            "#,
        )?;

        let mut count = 0;
        for obs in observations {
            stmt.execute(params![
                obs.instrument.storage_key(),
                obs.date.format("%Y-%m-%d").to_string(),
                obs.close,
                obs.source
            ])?;
            count += 1;
        }

        drop(stmt);
        tx.commit()?;
        Ok(count)
    }

    pub fn seed_knowledge_chunks(&self, chunks: &[(&str, &str, &str)]) -> Result<()> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM knowledge_chunks", [], |r| r.get(0))?;
        if count > 0 {
            return Ok(());
        }
        for &(title, tags, body) in chunks {
            self.conn.execute(
                "INSERT INTO knowledge_chunks (title, tags, body) VALUES (?1, ?2, ?3)",
                params![title, tags, body],
            )?;
        }
        Ok(())
    }

    pub fn load_knowledge_chunks(&self, instrument_tags: &[&str]) -> Result<Vec<String>> {
        let placeholders: Vec<String> = (0..instrument_tags.len())
            .map(|i| format!("tags LIKE ?{}", i + 2))
            .collect();
        let mut where_clause = "tags LIKE ?1".to_owned();
        for p in &placeholders {
            where_clause.push_str(" OR ");
            where_clause.push_str(p);
        }
        let sql = format!(
            "SELECT title, body FROM knowledge_chunks WHERE {where_clause} ORDER BY id ASC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut param_values: Vec<String> = vec!["%all%".to_owned()];
        for tag in instrument_tags {
            param_values.push(format!("%{tag}%"));
        }
        let params: Vec<&dyn rusqlite::types::ToSql> = param_values
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt.query_map(&*params, |row| {
            let title: String = row.get(0)?;
            let body: String = row.get(1)?;
            Ok(format!("## {title}\n\n{body}"))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Persist one inference. The four `hypothesis_*` / `overlay_*`
    /// arguments are optional because the parser may fail (or because
    /// older code paths may not provide them). They are stored as JSON
    /// arrays for the list-shaped fields and plain text for the rest.
    #[allow(clippy::too_many_arguments)]
    pub fn save_inference(
        &self,
        provider: &str,
        model: &str,
        system_prompt: &str,
        user_message: &str,
        response: &str,
        vix_close: Option<f64>,
        vix_level: Option<&str>,
        hypothesis_question: Option<&str>,
        hypothesis_outcomes: Option<&[String]>,
        hypothesis_context: Option<&str>,
        overlay_instruments: Option<&[String]>,
    ) -> Result<i64> {
        let now = chrono::Utc::now().to_rfc3339();
        let outcomes_json = hypothesis_outcomes
            .map(|o| serde_json::to_string(o).unwrap_or_else(|_| "[]".to_owned()));
        let overlay_json = overlay_instruments
            .map(|o| serde_json::to_string(o).unwrap_or_else(|_| "[]".to_owned()));
        self.conn.execute(
            r#"INSERT INTO ai_inferences
               (created_at, provider, model, system_prompt, user_message, response,
                vix_close, vix_level,
                hypothesis_question, hypothesis_outcomes, hypothesis_context, overlay_instruments)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"#,
            params![
                now, provider, model, system_prompt, user_message, response,
                vix_close, vix_level,
                hypothesis_question, outcomes_json, hypothesis_context, overlay_json,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn load_recent_inferences(&self, limit: usize) -> Result<Vec<SavedInference>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, provider, model, response, vix_close, vix_level, \
                    hypothesis_question, hypothesis_outcomes, hypothesis_context, overlay_instruments \
             FROM ai_inferences ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], row_to_saved_inference)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn load_inferences_in_range(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<SavedInference>> {
        let from_str = from.format("%Y-%m-%d").to_string();
        let to_str = (to + chrono::Duration::days(1)).format("%Y-%m-%d").to_string();
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, provider, model, response, vix_close, vix_level, \
                    hypothesis_question, hypothesis_outcomes, hypothesis_context, overlay_instruments \
             FROM ai_inferences WHERE created_at >= ?1 AND created_at < ?2 \
             ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(params![from_str, to_str], row_to_saved_inference)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn insert_alert_event(&self, event: &AlertEvent) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO alert_events (timestamp_utc, instrument, level, note)
            VALUES (?1, ?2, ?3, ?4)
            "#,
            params![
                event.timestamp_utc.to_rfc3339(),
                event.instrument.storage_key(),
                alert_level_to_str(event.level),
                event.note
            ],
        )?;

        Ok(())
    }

    pub fn clear_inferences(&self) -> Result<()> {
        self.conn.execute("DELETE FROM ai_inferences", [])?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // 51Folds model tracking — used by start_folds_create + the resume
    // sweep on App::new. The polling threads themselves do not go
    // through Storage (it is not Send); they open their own
    // Connection via the helper functions below this `impl`.
    // -----------------------------------------------------------------

    /// Load every row whose status is still `pending`. Used by the resume
    /// sweep on App::new.
    pub fn load_pending_folds_models(&self) -> Result<Vec<FoldsModelRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, model_id, status, created_at, completed_at, last_polled_at, question \
             FROM folds_models WHERE status = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(params![FOLDS_STATUS_PENDING], row_to_folds_record)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Load the full `response_json` for a 51Folds model linked to a
    /// given `ai_inferences` row. Returns `None` if no model exists for
    /// this inference or if the model hasn't completed yet.
    pub fn load_folds_response_for_inference(&self, inference_id: i64) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT response_json FROM folds_models WHERE inference_id = ?1 AND response_json IS NOT NULL LIMIT 1",
        )?;
        let mut rows = stmt.query(params![inference_id])?;
        if let Some(row) = rows.next()? {
            let json: String = row.get(0)?;
            Ok(Some(json))
        } else {
            Ok(None)
        }
    }

    /// Update a row's status (and completed_at if terminal). Used by the
    /// foreground when only one model is in flight via FoldsTask; the
    /// polling threads update via the standalone helper.
    pub fn update_folds_model_status(
        &self,
        model_id: &str,
        status: &str,
        completed_at: Option<DateTime<Utc>>,
    ) -> Result<()> {
        self.conn.execute(
            r#"UPDATE folds_models
               SET status = ?1,
                   completed_at = ?2,
                   last_polled_at = ?3
               WHERE model_id = ?4"#,
            params![
                status,
                completed_at.map(|t| t.to_rfc3339()),
                Utc::now().to_rfc3339(),
                model_id,
            ],
        )?;
        Ok(())
    }
}

/// Map one ai_inferences row to a `SavedInference`. The JSON-encoded
/// hypothesis_outcomes / overlay_instruments fields are decoded
/// best-effort: if decoding fails or the value is NULL we surface `None`
/// rather than failing the whole load.
fn row_to_saved_inference(row: &rusqlite::Row<'_>) -> rusqlite::Result<SavedInference> {
    let hypothesis_outcomes_raw: Option<String> = row.get(8)?;
    let overlay_raw: Option<String> = row.get(10)?;
    Ok(SavedInference {
        id: row.get(0)?,
        created_at: row.get(1)?,
        provider: row.get(2)?,
        model: row.get(3)?,
        response: row.get(4)?,
        vix_close: row.get(5)?,
        vix_level: row.get(6)?,
        hypothesis_question: row.get(7)?,
        hypothesis_outcomes: hypothesis_outcomes_raw
            .as_deref()
            .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok()),
        hypothesis_context: row.get(9)?,
        overlay_instruments: overlay_raw
            .as_deref()
            .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok()),
    })
}

fn row_to_folds_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<FoldsModelRecord> {
    let created_str: String = row.get(3)?;
    let completed_str: Option<String> = row.get(4)?;
    let polled_str: Option<String> = row.get(5)?;
    Ok(FoldsModelRecord {
        id: row.get(0)?,
        model_id: row.get(1)?,
        status: row.get(2)?,
        created_at: parse_rfc3339(&created_str),
        completed_at: completed_str.as_deref().map(parse_rfc3339),
        last_polled_at: polled_str.as_deref().map(parse_rfc3339),
        question: row.get(6)?,
    })
}

fn parse_rfc3339(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

/// Standalone helper for the polling threads: open a fresh `Connection`
/// to the same database file and update one model's status. SQLite WAL
/// mode (enabled in `init`) makes concurrent writes from multiple
/// connections safe. Failures are logged via `eprintln!` but never
/// propagated — the polling thread should keep going so the next cycle
/// has another chance to persist.
pub fn update_folds_model_status_standalone(
    db_path: &Path,
    model_id: &str,
    status: &str,
    completed_at: Option<DateTime<Utc>>,
) {
    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("warn: folds poll could not open db: {e:#}");
            return;
        }
    };
    let result = conn.execute(
        r#"UPDATE folds_models
           SET status = ?1,
               completed_at = ?2,
               last_polled_at = ?3
           WHERE model_id = ?4"#,
        params![
            status,
            completed_at.map(|t| t.to_rfc3339()),
            Utc::now().to_rfc3339(),
            model_id,
        ],
    );
    if let Err(e) = result {
        eprintln!("warn: folds poll status update failed for {model_id}: {e:#}");
    }
}

/// Standalone helper for persisting a completed 51Folds model response.
/// Updates status to success, stores the full response JSON, denormalized
/// outcomes, and short_summary. Called from the background polling thread.
pub fn update_folds_model_completed_standalone(
    db_path: &Path,
    model_id: &str,
    response_json: &str,
    outcomes_json: &str,
    short_summary: &str,
) {
    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("warn: folds completed update could not open db: {e:#}");
            return;
        }
    };
    let result = conn.execute(
        r#"UPDATE folds_models
           SET status = ?1,
               completed_at = ?2,
               last_polled_at = ?3,
               response_json = ?4,
               outcomes = ?5,
               short_summary = ?6
           WHERE model_id = ?7"#,
        params![
            crate::models::FOLDS_STATUS_SUCCESS,
            Utc::now().to_rfc3339(),
            Utc::now().to_rfc3339(),
            response_json,
            outcomes_json,
            short_summary,
            model_id,
        ],
    );
    if let Err(e) = result {
        eprintln!("warn: folds completed update failed for {model_id}: {e:#}");
    }
}

fn alert_level_to_str(level: AlertLevel) -> &'static str {
    match level {
        AlertLevel::Normal => "normal",
        AlertLevel::ApproachingExtreme => "approaching_extreme",
        AlertLevel::Extreme => "extreme",
    }
}

fn intern_source(s: &str) -> &'static str {
    match s {
        "FRED VIXCLS" => "FRED VIXCLS",
        "FRED PSOYBUSDM" => "FRED PSOYBUSDM",
        "Alpha Vantage GOLD" => "Alpha Vantage GOLD",
        "Alpha Vantage SILVER" => "Alpha Vantage SILVER",
        "Alpha Vantage BTC" => "Alpha Vantage BTC",
        "Alpha Vantage WTI" => "Alpha Vantage WTI",
        "Alpha Vantage NATURAL_GAS" => "Alpha Vantage NATURAL_GAS",
        "Alpha Vantage COPPER" => "Alpha Vantage COPPER",
        "Alpha Vantage ALUMINUM" => "Alpha Vantage ALUMINUM",
        "Alpha Vantage WHEAT" => "Alpha Vantage WHEAT",
        "Alpha Vantage CORN" => "Alpha Vantage CORN",
        _ => "Unknown",
    }
}
