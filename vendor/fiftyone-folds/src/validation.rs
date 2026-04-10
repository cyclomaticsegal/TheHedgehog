use crate::constants::{
    MAX_CONTEXT_WORDS, MAX_OUTCOMES, MIN_OUTCOMES, MIN_QUESTION_LENGTH, VALID_MODEL_TYPES,
    WARN_CONTEXT_WORDS,
};
use crate::errors::FoldsError;

/// Validate model creation params before sending to the API.
///
/// Collects all errors (not just the first). Emits a warning to stderr
/// if `additional_context` is under 250 words.
pub fn validate_create_model(
    question: &str,
    outcomes: &[String],
    additional_context: &str,
    model_type: &str,
) -> Result<(), FoldsError> {
    let mut errors: Vec<String> = Vec::new();

    if question.len() < MIN_QUESTION_LENGTH {
        errors.push(format!(
            "question: must be at least {} characters, got {}",
            MIN_QUESTION_LENGTH,
            question.len()
        ));
    }

    if outcomes.len() < MIN_OUTCOMES || outcomes.len() > MAX_OUTCOMES {
        errors.push(format!(
            "outcomes: must have {}-{} items, got {}",
            MIN_OUTCOMES,
            MAX_OUTCOMES,
            outcomes.len()
        ));
    }

    let word_count = additional_context.split_whitespace().count();
    if word_count > MAX_CONTEXT_WORDS {
        errors.push(format!(
            "additional_context: exceeds {}-word limit ({} words)",
            MAX_CONTEXT_WORDS, word_count
        ));
    } else if word_count < WARN_CONTEXT_WORDS {
        eprintln!(
            "[51folds] additional_context is short ({} words); {}+ recommended for driver quality",
            word_count, WARN_CONTEXT_WORDS
        );
    }

    if !VALID_MODEL_TYPES.contains(&model_type) {
        errors.push(format!(
            "type: must be one of {:?} (exact case), got '{}'",
            VALID_MODEL_TYPES, model_type
        ));
    }

    if !errors.is_empty() {
        return Err(FoldsError::Validation {
            message: format!("Client-side validation failed:\n  {}", errors.join("\n  ")),
            field_errors: Vec::new(),
            reasons: Vec::new(),
            status_code: None,
            body: None,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcomes(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("outcome {}", i)).collect()
    }

    #[test]
    fn valid_input_passes() {
        let ctx = "word ".repeat(260);
        let result =
            validate_create_model("A valid question here?", &outcomes(3), &ctx, "Advanced");
        assert!(result.is_ok());
    }

    #[test]
    fn question_too_short() {
        let result = validate_create_model("short", &outcomes(2), "context words here", "Advanced");
        assert!(result.is_err());
    }

    #[test]
    fn outcomes_too_few() {
        let result = validate_create_model(
            "A valid question here?",
            &outcomes(1),
            "context words here",
            "Advanced",
        );
        assert!(result.is_err());
    }

    #[test]
    fn outcomes_too_many() {
        let result = validate_create_model(
            "A valid question here?",
            &outcomes(6),
            "context words here",
            "Advanced",
        );
        assert!(result.is_err());
    }

    #[test]
    fn context_over_300_words() {
        let ctx = "word ".repeat(301);
        let result =
            validate_create_model("A valid question here?", &outcomes(2), &ctx, "Advanced");
        assert!(result.is_err());
    }

    #[test]
    fn invalid_type_lowercase() {
        let result = validate_create_model(
            "A valid question here?",
            &outcomes(2),
            "context words here",
            "advanced",
        );
        assert!(result.is_err());
    }

    #[test]
    fn invalid_type_unknown() {
        let result = validate_create_model(
            "A valid question here?",
            &outcomes(2),
            "context words here",
            "Premium",
        );
        assert!(result.is_err());
    }

    #[test]
    fn all_valid_types_accepted() {
        for t in &["Overview", "Insight", "Advanced"] {
            let result = validate_create_model(
                "A valid question here?",
                &outcomes(2),
                &"word ".repeat(260),
                t,
            );
            assert!(result.is_ok(), "type '{}' should be accepted", t);
        }
    }

    #[test]
    fn collects_multiple_errors() {
        let result = validate_create_model("short", &outcomes(0), &"word ".repeat(301), "bad");
        match result {
            Err(FoldsError::Validation { message, .. }) => {
                assert!(message.contains("question:"));
                assert!(message.contains("outcomes:"));
                assert!(message.contains("additional_context:"));
                assert!(message.contains("type:"));
            }
            _ => panic!("expected Validation error"),
        }
    }
}
