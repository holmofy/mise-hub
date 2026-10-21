use crate::types::ToolEntry;

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
}
