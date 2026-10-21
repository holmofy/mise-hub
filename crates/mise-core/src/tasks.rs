//! Task parsing and dependency graph helpers for `mise tasks ls --json`.
//!
//! The graph stores edges as `(task, dependency)`. `dependency_order`
//! returns a topological order where dependencies come first; missing
//! dependencies are kept in the result, and cycles terminate with the
//! cycle member recorded exactly once.

use crate::error::CoreError;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub name: String,
    pub aliases: Vec<String>,
    pub description: Option<String>,
    pub source: Option<PathBuf>,
    pub depends: Vec<String>,
    pub depends_post: Vec<String>,
    pub wait_for: Vec<String>,
    pub hide: bool,
    pub global: bool,
    pub run: Vec<String>,
    pub file: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskGraph {
    pub nodes: Vec<Task>,
    /// `(task, dependency)` pairs from `depends` and `depends_post`.
    /// `wait_for` is intentionally NOT an edge: it only delays a task's
    /// start until the dependency started, without ordering completion.
    pub edges: Vec<(String, String)>,
}

#[derive(Deserialize)]
struct RawTask {
    name: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    source: Option<PathBuf>,
    #[serde(default)]
    depends: Vec<String>,
    #[serde(default)]
    depends_post: Vec<String>,
    #[serde(default)]
    wait_for: Vec<String>,
    #[serde(default)]
    hide: bool,
    #[serde(default)]
    global: bool,
    #[serde(default)]
    run: Vec<String>,
    #[serde(default)]
    file: Option<PathBuf>,
}

/// Parse the rows of `mise tasks ls --json`.
pub fn parse_tasks(json: &str) -> Result<Vec<Task>, CoreError> {
    let raw: Vec<RawTask> = serde_json::from_str(json).map_err(CoreError::Json)?;
    Ok(raw
        .into_iter()
        .map(|r| Task {
            name: r.name,
            aliases: r.aliases,
            description: r.description,
            source: r.source,
            depends: r.depends,
            depends_post: r.depends_post,
            wait_for: r.wait_for,
            hide: r.hide,
            global: r.global,
            run: r.run,
            file: r.file,
        })
        .collect())
}

/// Build the dependency graph: one edge per entry in `depends` and
/// `depends_post`, as `(task, dependency)`.
pub fn build_graph(tasks: Vec<Task>) -> TaskGraph {
    let mut edges = Vec::new();
    for t in &tasks {
        for dep in t.depends.iter().chain(t.depends_post.iter()) {
            edges.push((t.name.clone(), dep.clone()));
        }
    }
    TaskGraph {
        nodes: tasks,
        edges,
    }
}

/// Task names needed to run `task`, dependencies first, then the task
/// itself. Missing dependencies stay in the result (the UI should flag
/// them). A cycle terminates: each task is visited at most once, and the
/// cycle member appears once in the returned order.
pub fn dependency_order(graph: &TaskGraph, task: &str) -> Vec<String> {
    let mut deps: HashMap<&str, Vec<&str>> = HashMap::new();
    for (t, d) in &graph.edges {
        deps.entry(t.as_str()).or_default().push(d.as_str());
    }
    let mut order = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    // True while the node is on the current DFS path: a revisit marks a
    // cycle — the node is recorded once and the descent stops, so there
    // is no panic and no infinite loop.
    let mut on_stack: HashSet<String> = HashSet::new();

    fn visit(
        node: &str,
        deps: &HashMap<&str, Vec<&str>>,
        order: &mut Vec<String>,
        visited: &mut HashSet<String>,
        on_stack: &mut HashSet<String>,
    ) {
        if visited.contains(node) {
            return;
        }
        if on_stack.contains(node) {
            // Cycle: record the member once so the runner can attempt it
            // and fail visibly, instead of dropping it silently.
            visited.insert(node.to_string());
            order.push(node.to_string());
            return;
        }
        on_stack.insert(node.to_string());
        if let Some(list) = deps.get(node) {
            for d in list.clone() {
                visit(d, deps, order, visited, on_stack);
            }
        }
        on_stack.remove(node);
        if !visited.contains(node) {
            visited.insert(node.to_string());
            order.push(node.to_string());
        }
    }

    visit(task, &deps, &mut order, &mut visited, &mut on_stack);
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_TASK: &str = r#"{
        "name": "build", "aliases": ["b"], "description": "Build the project",
        "source": "/proj/mise.toml", "config_sources": ["/proj/mise.toml"],
        "depends": ["lint"], "depends_post": ["notify"], "wait_for": ["setup"],
        "env": [], "dir": "/proj", "hide": false, "global": false,
        "raw": false, "interactive": false, "sources": [], "outputs": [],
        "run": ["echo build"], "file": null, "timeout": null,
        "tools": {}, "usage": ""
    }"#;

    #[test]
    fn parse_full_task() {
        let t = &parse_tasks(&format!("[{FULL_TASK}]")).unwrap()[0];
        assert_eq!(t.name, "build");
        assert_eq!(t.aliases, vec!["b"]);
        assert_eq!(t.description.as_deref(), Some("Build the project"));
        assert_eq!(
            t.source.as_deref(),
            Some(std::path::Path::new("/proj/mise.toml"))
        );
        assert_eq!(t.depends, vec!["lint"]);
        assert_eq!(t.depends_post, vec!["notify"]);
        assert_eq!(t.wait_for, vec!["setup"]);
        assert!(!t.hide);
        assert!(!t.global);
        assert_eq!(t.run, vec!["echo build"]);
        assert_eq!(t.file, None);
    }

    #[test]
    fn parse_missing_fields_use_defaults() {
        let t = &parse_tasks(r#"[{"name": "test"}]"#).unwrap()[0];
        assert_eq!(t.aliases, Vec::<String>::new());
        assert_eq!(t.description, None);
        assert_eq!(t.source, None);
        assert_eq!(t.depends, Vec::<String>::new());
        assert_eq!(t.depends_post, Vec::<String>::new());
        assert_eq!(t.wait_for, Vec::<String>::new());
        assert!(!t.hide);
        assert!(!t.global);
        assert_eq!(t.run, Vec::<String>::new());
        assert_eq!(t.file, None);
    }

    #[test]
    fn parse_invalid_json_is_error_not_panic() {
        assert!(parse_tasks("not json").is_err());
        assert!(parse_tasks("{\"name\": \"x\"}").is_err()); // object, not array
    }

    #[test]
    fn parse_empty_array() {
        assert!(parse_tasks("[]").unwrap().is_empty());
    }

    #[test]
    fn parse_hides_and_globals() {
        let ts = parse_tasks(r#"[{"name":"a","hide":true},{"name":"b","global":true}]"#).unwrap();
        assert!(ts[0].hide);
        assert!(ts[1].global);
    }

    fn graph(edges: &[(&str, &[&str])]) -> TaskGraph {
        let nodes = edges
            .iter()
            .map(|(n, deps)| Task {
                name: (*n).into(),
                aliases: vec![],
                description: None,
                source: None,
                depends: (*deps).iter().map(|d| (*d).into()).collect(),
                depends_post: vec![],
                wait_for: vec![],
                hide: false,
                global: false,
                run: vec![],
                file: None,
            })
            .collect();
        build_graph(nodes)
    }

    #[test]
    fn graph_edges_from_depends_and_depends_post() {
        let g = graph(&[("build", &["lint"]), ("check", &[])]);
        let mut e = g.edges.clone();
        e.sort();
        assert_eq!(e, vec![("build".to_string(), "lint".to_string())]);
    }

    #[test]
    fn graph_edges_include_depends_post() {
        let g = build_graph(vec![Task {
            name: "build".into(),
            aliases: vec![],
            description: None,
            source: None,
            depends: vec!["lint".into()],
            depends_post: vec!["notify".into()],
            wait_for: vec!["setup".into()],
            hide: false,
            global: false,
            run: vec![],
            file: None,
        }]);
        let mut e = g.edges.clone();
        e.sort();
        assert_eq!(
            e,
            vec![
                ("build".to_string(), "lint".to_string()),
                ("build".to_string(), "notify".to_string())
            ]
        );
        // wait_for is documented as NOT producing an edge: it delays start,
        // it does not order completion, so including it would make
        // `dependency_order` run tasks the task does not actually need.
        assert!(!e.contains(&("build".to_string(), "setup".to_string())));
    }

    #[test]
    fn order_linear_chain_test_then_build_then_lint() {
        // `test -> build -> lint`: run lint, then build, then test.
        let g = graph(&[("test", &["build"]), ("build", &["lint"]), ("lint", &[])]);
        assert_eq!(dependency_order(&g, "test"), vec!["lint", "build", "test"]);
    }

    #[test]
    fn order_diamond_no_duplicates() {
        // test -> {lint, format} -> build
        let g = graph(&[
            ("test", &["lint", "format"]),
            ("lint", &["build"]),
            ("format", &["build"]),
            ("build", &[]),
        ]);
        let order = dependency_order(&g, "test");
        assert_eq!(order, vec!["build", "lint", "format", "test"]);
    }

    #[test]
    fn order_missing_dependency_is_kept() {
        let g = graph(&[("test", &["nope"]), ("lint", &[])]);
        assert_eq!(dependency_order(&g, "test"), vec!["nope", "test"]);
    }

    #[test]
    fn order_unknown_task_returns_just_that_task() {
        let g = graph(&[("lint", &[])]);
        assert_eq!(dependency_order(&g, "ghost"), vec!["ghost"]);
    }

    #[test]
    fn order_cycle_terminates_and_records_once() {
        // c -> a -> b -> a: `c` depends on a task cycle it can never finish.
        let g = graph(&[("c", &["a"]), ("a", &["b"]), ("b", &["a"])]);
        let order = dependency_order(&g, "c");
        // Every node appears exactly once; cycle members are present.
        let mut sorted = order.clone();
        sorted.sort();
        assert_eq!(sorted, vec!["a", "b", "c"]);
        // `c` is last, no duplicates anywhere.
        assert_eq!(order.last().map(String::as_str), Some("c"));
        assert_eq!(order.len(), order.iter().collect::<HashSet<_>>().len());
    }

    #[test]
    fn order_self_cycle_terminates() {
        let g = graph(&[("a", &["a"])]);
        assert_eq!(dependency_order(&g, "a"), vec!["a"]);
    }
}
