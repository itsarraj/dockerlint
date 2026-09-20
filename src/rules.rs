//! The five anti-pattern checks this tool exists to catch. Each one is
//! independently testable against a small hand-built instruction list,
//! and `lib.rs` also runs the whole pipeline against two full fixture
//! Dockerfiles (one hitting every rule, one clean).

use std::collections::HashSet;

use serde::Serialize;

use crate::parser::Instruction;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub rule: &'static str,
    pub severity: Severity,
    pub line: usize,
    pub message: String,
}

const ARCHIVE_EXTENSIONS: &[&str] = &[
    ".tar", ".tar.gz", ".tgz", ".tar.bz2", ".tbz2", ".tar.xz", ".txz", ".tar.zst",
];

/// Runs every rule over a parsed Dockerfile and returns findings sorted
/// by line number (stable order for both human and JSON output).
pub fn lint(instructions: &[Instruction]) -> Vec<Finding> {
    let mut findings = Vec::new();
    findings.extend(check_unpinned_base_image(instructions));
    findings.extend(check_add_instead_of_copy(instructions));
    findings.extend(check_apt_get_install(instructions));

    let last_stage = last_stage_instructions(instructions);
    findings.extend(check_root_user(&last_stage));
    findings.extend(check_missing_healthcheck(&last_stage));

    findings.sort_by_key(|f| f.line);
    findings
}

/// Splits `instructions` at each `FROM` and returns only the final
/// stage's slice — the one that's actually shipped as the resulting
/// image, and so the only stage where "no USER" or "no HEALTHCHECK"
/// matters. Earlier build-only stages routinely skip both on purpose.
fn last_stage_instructions(instructions: &[Instruction]) -> Vec<Instruction> {
    let mut last_from_idx = None;
    for (idx, inst) in instructions.iter().enumerate() {
        if inst.keyword == "FROM" {
            last_from_idx = Some(idx);
        }
    }
    match last_from_idx {
        Some(idx) => instructions[idx..].to_vec(),
        None => instructions.to_vec(),
    }
}

/// Rule: base image tag must be pinned — no `FROM ubuntu` (defaults to
/// whatever `latest` happens to resolve to today) and no explicit
/// `FROM ubuntu:latest` either, since that's the exact same footgun
/// spelled out. A `FROM` that names an earlier build stage (`FROM
/// builder AS final`) or `scratch` is not a registry image and is
/// skipped, and so is anything containing a `$` build-arg variable this
/// tool has no way to resolve statically.
fn check_unpinned_base_image(instructions: &[Instruction]) -> Vec<Finding> {
    let mut stage_names: HashSet<String> = HashSet::new();
    let mut findings = Vec::new();

    for inst in instructions {
        if inst.keyword != "FROM" {
            continue;
        }
        let tokens: Vec<&str> = inst.args.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        let image_ref = tokens[0];

        // Record this stage's own name (if any) before evaluating the
        // image ref, so a *later* stage can reference it, but a stage
        // can't accidentally skip its own check by naming itself the
        // same as its base image.
        let this_stage_name = if tokens.len() >= 3 && tokens[1].eq_ignore_ascii_case("as") {
            Some(tokens[2].to_string())
        } else {
            None
        };

        let skip = image_ref.eq_ignore_ascii_case("scratch")
            || image_ref.contains('$')
            || image_ref.contains('@') // pinned by digest, stricter than a tag
            || stage_names.contains(image_ref);

        if !skip {
            let last_segment = image_ref.rsplit('/').next().unwrap_or(image_ref);
            match last_segment.split_once(':') {
                None => findings.push(Finding {
                    rule: "unpinned-base-image",
                    severity: Severity::Warning,
                    line: inst.line,
                    message: format!(
                        "base image '{image_ref}' has no tag — pin an explicit version (e.g. '{image_ref}:22.04') instead of floating on whatever 'latest' resolves to today"
                    ),
                }),
                Some((_, "latest")) => findings.push(Finding {
                    rule: "unpinned-base-image",
                    severity: Severity::Warning,
                    line: inst.line,
                    message: format!(
                        "base image '{image_ref}' is explicitly pinned to 'latest', which is exactly as unreproducible as no tag at all"
                    ),
                }),
                Some(_) => {}
            }
        }

        if let Some(name) = this_stage_name {
            stage_names.insert(name);
        }
    }

    findings
}

/// Rule: `ADD` should only be used for its two genuinely special
/// features — fetching a remote URL, or auto-extracting a local
/// archive. Anything else (a local file or directory) should be `COPY`,
/// which is the same operation with none of `ADD`'s surprising
/// behavior (e.g. silently extracting a `.tar.gz` you didn't want
/// extracted).
fn check_add_instead_of_copy(instructions: &[Instruction]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for inst in instructions {
        if inst.keyword != "ADD" {
            continue;
        }
        let tokens: Vec<&str> = inst
            .args
            .split_whitespace()
            .filter(|t| !t.starts_with("--"))
            .collect();
        // Last token is the destination; everything else is a source.
        if tokens.len() < 2 {
            continue;
        }
        let sources = &tokens[..tokens.len() - 1];
        let justified = sources.iter().any(|src| {
            src.starts_with("http://")
                || src.starts_with("https://")
                || ARCHIVE_EXTENSIONS.iter().any(|ext| src.ends_with(ext))
        });
        if !justified {
            findings.push(Finding {
                rule: "add-instead-of-copy",
                severity: Severity::Warning,
                line: inst.line,
                message: format!(
                    "ADD used for a local path ('{}') that isn't a URL or an archive to auto-extract — use COPY instead",
                    sources.join(" ")
                ),
            });
        }
    }
    findings
}

/// Rule: the final stage should drop root before running, via an
/// explicit non-root `USER`. Both "no USER instruction at all" and "USER
/// root" (or its numeric UID `0`) are flagged as the same underlying
/// problem, with a message naming which one applies.
fn check_root_user(last_stage: &[Instruction]) -> Vec<Finding> {
    let user_insts: Vec<&Instruction> = last_stage.iter().filter(|i| i.keyword == "USER").collect();
    match user_insts.last() {
        None => {
            let line = last_stage.first().map(|i| i.line).unwrap_or(1);
            vec![Finding {
                rule: "runs-as-root",
                severity: Severity::Warning,
                line,
                message: "no USER instruction — the container runs as root by default; add a USER instruction that switches to an unprivileged account".to_string(),
            }]
        }
        Some(inst) => {
            let user = inst.args.split_whitespace().next().unwrap_or("");
            let (name, _group) = user.split_once(':').unwrap_or((user, ""));
            if name == "root" || name == "0" {
                vec![Finding {
                    rule: "runs-as-root",
                    severity: Severity::Warning,
                    line: inst.line,
                    message: format!(
                        "USER is explicitly set to '{name}' — switch to an unprivileged account"
                    ),
                }]
            } else {
                Vec::new()
            }
        }
    }
}

/// Rule: the final stage should declare a `HEALTHCHECK` so an
/// orchestrator can tell "running" from "actually serving traffic"
/// apart. `HEALTHCHECK NONE` is treated as a deliberate, informed
/// opt-out — not flagged — since it's a real, intentional statement
/// rather than an oversight.
fn check_missing_healthcheck(last_stage: &[Instruction]) -> Vec<Finding> {
    let has_healthcheck = last_stage.iter().any(|i| i.keyword == "HEALTHCHECK");
    if has_healthcheck {
        return Vec::new();
    }
    let line = last_stage.first().map(|i| i.line).unwrap_or(1);
    vec![Finding {
        rule: "no-healthcheck",
        severity: Severity::Warning,
        line,
        message: "no HEALTHCHECK instruction — an orchestrator has no way to tell a hung container from a healthy one".to_string(),
    }]
}

/// Rule: an `apt-get install` (or `apt install`) should pass
/// `--no-install-recommends` (skips a real amount of dead weight most
/// images never use) and should be followed, in the *same* `RUN`, by
/// `rm -rf /var/lib/apt/lists/*` (otherwise the package index sits in
/// its own layer forever, since a later `RUN rm` only removes the files
/// from the final filesystem view, not the layer that already
/// committed them). Both are checked independently and can both fire on
/// the same line.
fn check_apt_get_install(instructions: &[Instruction]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for inst in instructions {
        if inst.keyword != "RUN" {
            continue;
        }
        let is_apt_install =
            inst.args.contains("apt-get install") || inst.args.contains("apt install");
        if !is_apt_install {
            continue;
        }
        if !inst.args.contains("--no-install-recommends") {
            findings.push(Finding {
                rule: "apt-missing-no-install-recommends",
                severity: Severity::Warning,
                line: inst.line,
                message: "apt-get install without --no-install-recommends pulls in extra packages most images never use".to_string(),
            });
        }
        if !inst.args.contains("rm -rf /var/lib/apt/lists/*") {
            findings.push(Finding {
                rule: "apt-missing-list-cleanup",
                severity: Severity::Warning,
                line: inst.line,
                message: "apt-get install not followed by 'rm -rf /var/lib/apt/lists/*' in the same RUN leaves the package index baked into a layer forever".to_string(),
            });
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn insts(src: &str) -> Vec<Instruction> {
        parse(src)
    }

    #[test]
    fn flags_from_with_no_tag() {
        let f = check_unpinned_base_image(&insts("FROM ubuntu\n"));
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rule, "unpinned-base-image");
    }

    #[test]
    fn flags_from_with_explicit_latest_tag() {
        let f = check_unpinned_base_image(&insts("FROM ubuntu:latest\n"));
        assert_eq!(f.len(), 1);
    }

    #[test]
    fn does_not_flag_a_properly_pinned_tag() {
        let f = check_unpinned_base_image(&insts("FROM ubuntu:22.04\n"));
        assert!(f.is_empty());
    }

    #[test]
    fn does_not_flag_scratch() {
        let f = check_unpinned_base_image(&insts("FROM scratch\n"));
        assert!(f.is_empty());
    }

    #[test]
    fn does_not_flag_digest_pinned_image() {
        let f = check_unpinned_base_image(&insts(
            "FROM ubuntu@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
        ));
        assert!(f.is_empty());
    }

    #[test]
    fn does_not_flag_a_reference_to_an_earlier_build_stage() {
        let f = check_unpinned_base_image(&insts(
            "FROM golang:1.21 AS builder\nFROM builder AS final\n",
        ));
        assert!(
            f.is_empty(),
            "referencing 'builder' as a base isn't a registry image: {f:?}"
        );
    }

    #[test]
    fn does_not_flag_an_unresolvable_build_arg() {
        let f = check_unpinned_base_image(&insts("FROM ${BASE_IMAGE}\n"));
        assert!(f.is_empty());
    }

    #[test]
    fn flags_add_for_a_plain_local_file() {
        let f = check_add_instead_of_copy(&insts("ADD config.json /app/config.json\n"));
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rule, "add-instead-of-copy");
    }

    #[test]
    fn does_not_flag_add_for_a_remote_url() {
        let f = check_add_instead_of_copy(&insts(
            "ADD https://example.com/data.tar.gz /app/data.tar.gz\n",
        ));
        assert!(f.is_empty());
    }

    #[test]
    fn does_not_flag_add_for_a_local_archive_that_gets_auto_extracted() {
        let f = check_add_instead_of_copy(&insts("ADD bundle.tar.gz /app/\n"));
        assert!(f.is_empty());
    }

    #[test]
    fn flags_missing_user_instruction() {
        let stage = insts("FROM ubuntu:22.04\nCMD [\"true\"]\n");
        let f = check_root_user(&stage);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rule, "runs-as-root");
    }

    #[test]
    fn flags_explicit_user_root() {
        let stage = insts("FROM ubuntu:22.04\nUSER root\n");
        let f = check_root_user(&stage);
        assert_eq!(f.len(), 1);
    }

    #[test]
    fn does_not_flag_a_non_root_user() {
        let stage = insts("FROM ubuntu:22.04\nUSER appuser\n");
        assert!(check_root_user(&stage).is_empty());
    }

    #[test]
    fn only_the_last_user_instruction_in_a_stage_counts() {
        // USER root then switching back to non-root later is fine —
        // what matters is what the container actually runs as.
        let stage = insts("FROM ubuntu:22.04\nUSER root\nUSER appuser\n");
        assert!(check_root_user(&stage).is_empty());
    }

    #[test]
    fn flags_missing_healthcheck() {
        let stage = insts("FROM ubuntu:22.04\nCMD [\"true\"]\n");
        assert_eq!(check_missing_healthcheck(&stage).len(), 1);
    }

    #[test]
    fn healthcheck_none_is_a_deliberate_opt_out_not_flagged() {
        let stage = insts("FROM ubuntu:22.04\nHEALTHCHECK NONE\n");
        assert!(check_missing_healthcheck(&stage).is_empty());
    }

    #[test]
    fn flags_apt_install_missing_both_recommends_and_cleanup() {
        let f = check_apt_get_install(&insts("RUN apt-get update && apt-get install -y curl\n"));
        assert_eq!(f.len(), 2);
        assert!(f
            .iter()
            .any(|x| x.rule == "apt-missing-no-install-recommends"));
        assert!(f.iter().any(|x| x.rule == "apt-missing-list-cleanup"));
    }

    #[test]
    fn does_not_flag_a_well_formed_apt_install() {
        let f = check_apt_get_install(&insts(
            "RUN apt-get update && apt-get install -y --no-install-recommends curl && rm -rf /var/lib/apt/lists/*\n",
        ));
        assert!(f.is_empty());
    }

    #[test]
    fn only_the_final_stage_is_checked_for_user_and_healthcheck() {
        let all = insts("FROM golang:1.21 AS builder\nRUN go build -o app .\nFROM gcr.io/distroless/base:latest\nCOPY --from=builder /app /app\nUSER nonroot\nHEALTHCHECK CMD [\"/app\", \"-health\"]\n");
        let findings = lint(&all);
        // The builder stage has no USER/HEALTHCHECK but shouldn't be
        // checked for either — only its unpinned ':latest' final-stage
        // tag should show up.
        assert!(!findings.iter().any(|f| f.rule == "runs-as-root"));
        assert!(!findings.iter().any(|f| f.rule == "no-healthcheck"));
        assert!(findings.iter().any(|f| f.rule == "unpinned-base-image"));
    }
}
