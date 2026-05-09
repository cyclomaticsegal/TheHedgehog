# Message to leave on the Dexter repo

**Suggested venue:** GitHub Discussions on `virattt/dexter` (under "Show and tell" or "General"). If Discussions aren't enabled, open an Issue with `[show-and-tell]` in the title so it doesn't read as a bug report.

---

**Title:** Vendored Dexter into a desktop app (The Hedgehog) — wanted to flag it

Hi Virat,

Quick heads up. I've built a Rust desktop app called **The Hedgehog** that vendors Dexter as a first-class tab.

The app is single-purpose: it watches what commodities and Bitcoin do when the VIX spikes, runs LLM regime analysis over the live data, and pushes the resulting hypotheses into [51Folds](https://51folds.ai) for Bayesian causal modelling. Dexter is layer four (the research agent). I added a `/51folds` slash command that synthesises a research conversation into a structured hypothesis and feeds it straight into the model-creation pipeline.

The terminal theme and voice are the Hedgehog's, but underneath it is Dexter doing the heavy research lift, with full attribution in the README and `docs/vendor-integrations/`.

- Repo: https://github.com/cyclomaticsegal/TheHedgehog
- Backstory: https://github.com/cyclomaticsegal/TheHedgehog/blob/main/docs/published/announcement.md

I tried reaching you on X first but didn't see it land, so leaving a note here too. Wanted you to see it before the Substack write-up goes out. If anything about the integration, attribution, or licensing posture needs adjusting, tell me and I'll fix it the same day.

Thanks for building Dexter. It saved me from writing the research layer myself.

— Simon Segal (cyclomaticsegal)
