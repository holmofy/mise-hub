//! Task parsing and dependency graph helpers for `mise tasks ls --json`.
//!
//! `ends`/`post_edges` both store `(task, dependency)`. `dependency_order`
//! returns an execution order: the task's `depends` chain first, then the
//! task itself, then its `depends_post` deps (mise runs post deps AFTER the
//! task, even when the task failed). Missing dependencies are kept in the
//! result, and cycles terminate with the cycle member recorded exactly once.
//!
//! `wait_for` is NOT part of the graph: it delays a task's start until the
//! dependency has STARTED, but adds no task to the run (mise does not run
//! the wait_for dependency), so it must not appear in the execution order.

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
    /// `(task, dependency)` pairs from `depends`: the dependency runs BEFORE
    /// the task (dependency_order chains these first). `wait_for` is
    /// intentionally NOT an edge: it only delays a task's start until the
    /// dependency started, without running it inside this run, so ordering
    /// completion by it would run tasks the task does not actually need.
    pub edges: Vec<(String, String)>,
    /// `(task, dependency)` pairs from `depends_post`: the dependency runs
    /// AFTER the task completes — even when the task FAILED — so the
    /// execution order trails these behind the task.
    pub post_edges: Vec<(String, String)>,
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

/// Build the dependency graph: one pre edge per entry in `depends` and one
/// post edge per entry in `depends_post`, each as `(task, dependency)`.
pub fn build_graph(tasks: Vec<Task>) -> TaskGraph {
    let mut edges = Vec::new();
    let mut post_edges = Vec::new();
    for t in &tasks {
        for dep in &t.depends {
            edges.push((t.name.clone(), dep.clone()));
        }
        for dep in &t.depends_post {
            post_edges.push((t.name.clone(), dep.clone()));
        }
    }
    TaskGraph {
        nodes: tasks,
        edges,
        post_edges,
    }
}

/// Execution order for running `task`: the `depends` chain first (each dep
/// preceded by its own chain), then `task`, then the `depends_post` trail
/// (each post dep with its own chain and its own post deps, since mise runs
/// post deps after the task — even on failure). Missing dependencies stay
/// in the result (the UI should flag them). A cycle terminates: each task
/// is visited at most once, and the cycle member appears once in the
/// returned order.
pub fn dependency_order(graph: &TaskGraph, task: &str) -> Vec<String> {
    let mut deps: HashMap<&str, Vec<&str>> = HashMap::new();
    for (t, d) in &graph.edges {
        deps.entry(t.as_str()).or_default().push(d.as_str());
    }
    let mut post: HashMap<&str, Vec<&str>> = HashMap::new();
    for (t, d) in &graph.post_edges {
        post.entry(t.as_str()).or_default().push(d.as_str());
    }

    let mut walk = Walk {
        deps: &deps,
        order: Vec::new(),
        visited: HashSet::new(),
        // True while the node is on the current DFS path: a revisit marks a
        // cycle — the node is recorded once and the descent stops, so there
        // is no panic and no infinite loop.
        on_stack: HashSet::new(),
    };

    walk.visit(task);
    // Post trail: after the task, each direct and nested post dep in
    // declared order, each with its own depends chain resolved first.
    let mut pending: Vec<&str> = post.get(task).map(|list| list.to_vec()).unwrap_or_default();
    let mut ix = 0;
    while ix < pending.len() {
        let node = pending[ix];
        ix += 1;
        walk.visit(node);
        if let Some(list) = post.get(node) {
            pending.extend(list.iter().copied());
        }
    }
    walk.order
}

/// DFS state shared by the pre-chain and post-trail walks so a node used in
/// several chains appears in the order exactly once (diamond shape).
struct Walk<'a> {
    deps: &'a HashMap<&'a str, Vec<&'a str>>,
    order: Vec<String>,
    visited: HashSet<String>,
    on_stack: HashSet<String>,
}

impl<'a> Walk<'a> {
    fn visit(&mut self, node: &str) {
        if self.visited.contains(node) {
            return;
        }
        if self.on_stack.contains(node) {
            // Cycle: record the member once so the runner can attempt it
            // and fail visibly, instead of dropping it silently.
            self.visited.insert(node.to_string());
            self.order.push(node.to_string());
            return;
        }
        self.on_stack.insert(node.to_string());
        if let Some(list) = self.deps.get(node) {
            for d in list.clone() {
                self.visit(d);
            }
        }
        self.on_stack.remove(node);
        if !self.visited.contains(node) {
            self.visited.insert(node.to_string());
            self.order.push(node.to_string());
        }
    }
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
    fn graph_edges_from_depends_only() {
        let g = graph(&[("build", &["lint"]), ("check", &[])]);
        assert_eq!(g.edges, vec![("build".to_string(), "lint".to_string())]);
        assert!(g.post_edges.is_empty());
    }

    #[test]
    fn graph_post_edges_carry_depends_post() {
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
        assert_eq!(g.edges, vec![("build".to_string(), "lint".to_string())]);
        assert_eq!(
            g.post_edges,
            vec![("build".to_string(), "notify".to_string())]
        );
        // wait_for is documented as NOT producing an edge (pre OR post): it
        // delays start until the dependency started, but adds no task to the
        // run, so including it would make `dependency_order` run tasks the
        // task does not actually need.
        assert!(
            !g.edges
                .contains(&("build".to_string(), "setup".to_string()))
        );
        assert!(
            !g.post_edges
                .contains(&("build".to_string(), "setup".to_string()))
        );
        assert!(!dependency_order(&g, "build").contains(&"setup".to_string()));
    }

    // mise runs post deps AFTER the task — even when the task fails — so
    // the order trail must put `notify` behind `build`, not in its deps.
    #[test]
    fn order_post_depends_run_after_task() {
        let mut g = graph(&[("build", &["lint"])]);
        g.post_edges = vec![("build".to_string(), "notify".to_string())];
        assert_eq!(
            dependency_order(&g, "build"),
            vec![
                "lint".to_string(),
                "build".to_string(),
                "notify".to_string()
            ]
        );
    }

    // A post dep keeps its OWN depends chain (resolved before it) and its
    // own post deps trail further behind; the shared visited set keeps
    // shared nodes from appearing twice.
    #[test]
    fn order_post_trail_is_recursive() {
        let mut g = graph(&[("build", &[]), ("notify", &["lookup"]), ("lookup", &[])]);
        g.post_edges = vec![
            ("build".to_string(), "notify".to_string()),
            ("notify".to_string(), "cleanup".to_string()),
        ];
        assert_eq!(
            dependency_order(&g, "build"),
            vec![
                "build".to_string(),
                "lookup".to_string(),
                "notify".to_string(),
                "cleanup".to_string()
            ]
        );
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
