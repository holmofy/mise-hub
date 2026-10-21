use crate::error::CoreError;
use crate::types::ToolEntry;

/// One row of the `mise search --all` plain-text table. Names keep any
/// backend prefix (`npm:`, `cargo:`, …) verbatim; `url` is the trailing
/// `http(s)://…` token when the description ends with one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeepSearchHit {
    pub name: String,
    pub description: String,
    pub url: Option<String>,
}

/// Parse the plain-text table printed by `mise search --no-header --all`.
/// Each non-empty line reads `name␣␣description` (run of 2+ spaces); the
/// description may end with a URL, which is moved to `url`. Lines without a
/// 2+ space separator are skipped; empty input yields an empty vec.
pub fn parse_search_table(text: &str) -> Result<Vec<DeepSearchHit>, CoreError> {
    let mut hits = Vec::new();
    for line in text.lines() {
        let mut parts = line.splitn(2, "  ");
        let (name, rest) = match (parts.next(), parts.next()) {
            (Some(name), Some(rest)) if !name.trim().is_empty() => (name.trim(), rest.trim()),
            _ => continue,
        };
        let (url, description) = split_trailing_url(rest);
        hits.push(DeepSearchHit {
            name: name.to_string(),
            description: description.to_string(),
            url,
        });
    }
    Ok(hits)
}

/// Pull the trailing `http(s)://…` token out of a table description when it
/// is the LAST token (preceded by whitespace). An earlier occurrence — e.g.
/// a Markdown link inside the text — stays part of the description.
fn split_trailing_url(rest: &str) -> (Option<String>, &str) {
    for pattern in ["https://", "http://"] {
        if let Some(idx) = rest.rfind(pattern) {
            let (before, candidate) = rest.split_at(idx);
            // Token boundary: whitespace immediately before the URL, and the
            // URL runs to the end of the description.
            if before.ends_with(char::is_whitespace) {
                return (Some(candidate.trim_end().to_string()), before.trim_end());
            }
        }
    }
    (None, rest)
}

pub struct SearchIndex {
    entries: Vec<ToolEntry>,
}

pub struct SearchHit {
    pub tool: ToolEntry,
    pub score: i32,
}

impl SearchIndex {
    pub fn new(entries: Vec<ToolEntry>) -> Self {
        Self { entries }
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        let q = query.trim().to_lowercase();
        let mut hits: Vec<SearchHit> = if q.is_empty() {
            self.entries
                .iter()
                .map(|e| SearchHit {
                    tool: e.clone(),
                    score: 0,
                })
                .collect()
        } else {
            self.entries
                .iter()
                .filter_map(|e| {
                    score(e, &q).map(|s| SearchHit {
                        tool: e.clone(),
                        score: s,
                    })
                })
                .collect()
        };
        hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.tool.short.cmp(&b.tool.short))
        });
        hits.truncate(limit);
        hits
    }
}

fn score(tool: &ToolEntry, q: &str) -> Option<i32> {
    let short = tool.short.to_lowercase();
    if short == q {
        return Some(1000);
    }
    if short.starts_with(q) {
        return Some(800);
    }
    if short.contains(q) {
        return Some(600);
    }
    if tool.aliases.iter().any(|a| a.to_lowercase() == q) {
        return Some(550);
    }
    if tool.aliases.iter().any(|a| a.to_lowercase().contains(q)) {
        return Some(500);
    }
    if tool.bins.iter().any(|b| b.to_lowercase() == q) {
        return Some(450);
    }
    if tool
        .description
        .as_deref()
        .is_some_and(|d| d.to_lowercase().contains(q))
    {
        return Some(300);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(short: &str, description: Option<&str>, aliases: &[&str], bins: &[&str]) -> ToolEntry {
        ToolEntry {
            short: short.into(),
            backends: vec![],
            bins: bins.iter().map(|s| s.to_string()).collect(),
            description: description.map(Into::into),
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn index() -> SearchIndex {
        SearchIndex::new(vec![
            entry("node", Some("JavaScript runtime"), &["nodejs"], &["node"]),
            entry("nodemon", Some("Monitor for node"), &[], &["nodemon"]),
            entry(
                "ripgrep",
                Some("recursively search directories"),
                &["rg"],
                &["rg"],
            ),
            entry("python", Some("Python language"), &[], &["python3"]),
        ])
    }

    #[test]
    fn exact_name_beats_prefix_and_contains() {
        let hits = index().search("node", 10);
        assert_eq!(hits[0].tool.short, "node");
        assert!(hits.iter().any(|h| h.tool.short == "nodemon"));
    }

    #[test]
    fn matches_alias_and_bin_and_description() {
        let idx = index();
        assert_eq!(idx.search("rg", 10)[0].tool.short, "ripgrep");
        assert_eq!(idx.search("python3", 10)[0].tool.short, "python");
        assert_eq!(idx.search("javascript", 10)[0].tool.short, "node");
    }

    #[test]
    fn empty_query_returns_all_alphabetically() {
        let hits = index().search("", 10);
        let names: Vec<_> = hits.iter().map(|h| h.tool.short.as_str()).collect();
        assert_eq!(names, vec!["node", "nodemon", "python", "ripgrep"]);
    }

    #[test]
    fn respects_limit_and_ignores_case() {
        let idx = index();
        assert_eq!(idx.search("NODE", 1).len(), 1);
        assert_eq!(idx.search("NODE", 1)[0].tool.short, "node");
    }

    #[test]
    fn backend_prefixed_names_are_not_split() {
        let idx = SearchIndex::new(vec![entry(
            "npm:prettier",
            Some("formatter"),
            &[],
            &["prettier"],
        )]);
        assert_eq!(idx.search("npm:prettier", 5)[0].tool.short, "npm:prettier");
    }

    #[test]
    fn deep_hits_parse_name_description_and_url() {
        let input = "ripgrep                           ripgrep \
recursively searches directories for a regex ... https://github.com/BurntSushi/ripgrep\n";
        let hits = parse_search_table(input).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "ripgrep");
        assert_eq!(
            hits[0].description,
            "ripgrep recursively searches directories for a regex ..."
        );
        assert_eq!(
            hits[0].url.as_deref(),
            Some("https://github.com/BurntSushi/ripgrep")
        );
    }

    #[test]
    fn deep_hits_keep_backend_prefix_verbatim() {
        let input = "npm:ripgrep                           [ripgrep](https://github.com) \
in a compact and cross-platform npm package.\n";
        let hits = parse_search_table(input).unwrap();
        assert_eq!(hits[0].name, "npm:ripgrep");
        assert!(
            hits[0]
                .description
                .contains("[ripgrep](https://github.com)")
        );
        assert_eq!(hits[0].url, None);
    }

    #[test]
    fn deep_hits_may_have_no_url() {
        let input = "cargo:ripgrep                     ripgrep is a \
line-oriented search tool ...\n";
        let hits = parse_search_table(input).unwrap();
        assert_eq!(hits[0].name, "cargo:ripgrep");
        assert_eq!(hits[0].url, None);
        assert!(hits[0].description.contains("line-oriented"));
    }

    #[test]
    fn deep_hits_skip_lines_without_separator_and_tolerate_empty_input() {
        // No 2+ space separator → skipped; empty → empty vec.
        assert_eq!(parse_search_table("").unwrap().len(), 0);
        assert_eq!(parse_search_table("\n \n").unwrap().len(), 0);
        assert_eq!(
            parse_search_table("justwords no separator here\n")
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn deep_hits_parse_all_four_line_shapes() {
        let input = "ripgrep                           ripgrep recursive search \
... https://github.com/BurntSushi/ripgrep\ncargo:ripgrep                     ripgrep \
is a line-oriented search tool ...\nnpm:ripgrep                       [ripgrep](https://github.com) \
compact cross-platform package.\naqua:BurntSushi/ripgrep           ripgrep recursive search ...\n";
        let hits = parse_search_table(input).unwrap();
        assert_eq!(hits.len(), 4);
        assert_eq!(hits[0].name, "ripgrep");
        assert_eq!(
            hits[0].url.as_deref(),
            Some("https://github.com/BurntSushi/ripgrep")
        );
        assert_eq!(hits[1].name, "cargo:ripgrep");
        assert_eq!(hits[1].url, None);
        assert_eq!(hits[2].name, "npm:ripgrep");
        assert_eq!(hits[3].name, "aqua:BurntSushi/ripgrep");
    }
}
