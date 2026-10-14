# Adversarial code review

Adapted from [awill1988/whip-it](https://github.com/awill1988/whip-it/tree/main/tools/adversarial_reviewer)
under the retained MIT license. Kimi evaluates concrete failure scenarios against
numbered diff evidence; this is separate from Markdown and commit-message linting.

Run `python3 -m unittest discover -s tools/adversarial_reviewer` for transport,
evidence, coverage, and disposition tests. With `KIMI_API_KEY` configured, run
`python3 tools/adversarial_reviewer/evaluate_model.py` for live qualification.

The publisher approves only complete clean reviews of the current PR commit.
Findings and unavailable reviews produce comments, never blocking request-changes
reviews. Binary changes, truncated coverage, and unqualified reviewers cannot
approve. Model agreement is not a proof that code is correct.
