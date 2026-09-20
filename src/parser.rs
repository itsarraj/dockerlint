//! A deliberately small Dockerfile parser — just enough structure for
//! `rules.rs` to reason about, not a full BuildKit-grade implementation.
//!
//! Comments (`#`) and blank lines are dropped, and backslash line
//! continuations are joined into one logical instruction before rules
//! ever see it, so a multi-line `RUN apt-get update && \` block is
//! inspected as a single string instead of tripping rules that scan for
//! flags "in the same command."

/// One logical Dockerfile instruction — a keyword (`FROM`, `RUN`, ...,
/// always upper-cased) plus the rest of the line as-is, and the 1-based
/// line number the instruction *started* on (used for reporting even
/// when continuation lines followed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    pub line: usize,
    pub keyword: String,
    pub args: String,
}

/// Parses raw Dockerfile text into a sequence of logical instructions.
///
/// Line-continuation handling: a line whose trimmed form ends in a
/// trailing `\` (and isn't itself inside a `#`-comment) is joined with
/// the next physical line, repeatedly, until a line without a trailing
/// backslash closes the instruction. This covers the overwhelming
/// majority of real Dockerfiles, which use the default `\` escape
/// character; the rarely-used `# escape=\`` directive switching to a
/// different escape char is not honored — see README.
pub fn parse(content: &str) -> Vec<Instruction> {
    let mut instructions = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        let trimmed = raw.trim();
        let start_line = i + 1; // 1-based
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }

        let mut logical = String::new();
        let mut cur = trimmed;
        loop {
            if let Some(stripped) = cur.strip_suffix('\\') {
                logical.push_str(stripped.trim_end());
                logical.push(' ');
                i += 1;
                if i >= lines.len() {
                    break;
                }
                cur = lines[i].trim();
                // A continuation line that's blank or a comment still
                // ends the instruction in real Dockerfiles' parser, but
                // in practice this almost never happens in valid files;
                // treat it as the end of the continuation to stay safe.
                if cur.is_empty() {
                    break;
                }
            } else {
                logical.push_str(cur);
                i += 1;
                break;
            }
        }

        let logical = logical.trim();
        let mut parts = logical.splitn(2, char::is_whitespace);
        let keyword = parts.next().unwrap_or("").to_uppercase();
        let args = parts.next().unwrap_or("").trim().to_string();
        if !keyword.is_empty() {
            instructions.push(Instruction {
                line: start_line,
                keyword,
                args,
            });
        }
    }
    instructions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_single_line_instructions() {
        let content = "FROM ubuntu:22.04\nRUN echo hi\n";
        let out = parse(content);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].keyword, "FROM");
        assert_eq!(out[0].args, "ubuntu:22.04");
        assert_eq!(out[0].line, 1);
        assert_eq!(out[1].keyword, "RUN");
        assert_eq!(out[1].args, "echo hi");
        assert_eq!(out[1].line, 2);
    }

    #[test]
    fn skips_blank_lines_and_comments() {
        let content = "# a comment\n\nFROM ubuntu:22.04\n\n# another\nRUN echo hi\n";
        let out = parse(content);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].line, 3);
        assert_eq!(out[1].line, 6);
    }

    #[test]
    fn joins_backslash_continuations_into_one_logical_instruction() {
        let content = "RUN apt-get update && \\\n    apt-get install -y curl && \\\n    rm -rf /var/lib/apt/lists/*\n";
        let out = parse(content);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].keyword, "RUN");
        assert!(out[0].args.contains("apt-get install -y curl"));
        assert!(out[0].args.contains("rm -rf /var/lib/apt/lists/*"));
        assert_eq!(
            out[0].line, 1,
            "reported line should be where the instruction started"
        );
    }

    #[test]
    fn keyword_is_uppercased_even_if_written_lowercase() {
        let content = "from ubuntu:22.04\n";
        let out = parse(content);
        assert_eq!(out[0].keyword, "FROM");
    }

    #[test]
    fn empty_file_produces_no_instructions() {
        assert!(parse("").is_empty());
        assert!(parse("\n\n\n").is_empty());
        assert!(parse("# just a comment\n").is_empty());
    }
}
