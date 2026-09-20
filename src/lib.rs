pub mod parser;
pub mod rules;

pub use rules::{Finding, Severity};

/// Parses and lints raw Dockerfile text in one call — the entry point
/// both the CLI and the fixture tests below use.
pub fn lint_dockerfile(content: &str) -> Vec<Finding> {
    let instructions = parser::parse(content);
    rules::lint(&instructions)
}

#[cfg(test)]
mod fixture_tests {
    use super::*;

    const BAD: &str = include_str!("../fixtures/bad.Dockerfile");
    const GOOD: &str = include_str!("../fixtures/good.Dockerfile");

    #[test]
    fn bad_fixture_trips_every_rule_at_least_once() {
        let findings = lint_dockerfile(BAD);
        let rules: std::collections::HashSet<&str> = findings.iter().map(|f| f.rule).collect();
        for expected in [
            "unpinned-base-image",
            "add-instead-of-copy",
            "runs-as-root",
            "no-healthcheck",
            "apt-missing-no-install-recommends",
            "apt-missing-list-cleanup",
        ] {
            assert!(
                rules.contains(expected),
                "expected rule '{expected}' to fire on the bad fixture, got: {rules:?}"
            );
        }
    }

    #[test]
    fn good_fixture_produces_zero_findings() {
        let findings = lint_dockerfile(GOOD);
        assert!(
            findings.is_empty(),
            "expected zero findings on the good fixture, got: {findings:?}"
        );
    }
}
