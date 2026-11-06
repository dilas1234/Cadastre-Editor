//! The editor's script is one file of top-level functions. A second function of the same name
//! silently replaces the first everywhere, with no error until that code runs.

use std::collections::HashSet;

#[test]
fn app_js_declares_each_top_level_function_once() {
    let js = include_str!("../web/app.js");
    let mut seen = HashSet::new();
    for line in js.lines() {
        let Some(rest) = line.strip_prefix("async function ").or_else(|| line.strip_prefix("function ")) else { continue };
        let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$').collect();
        assert!(seen.insert(name.clone()), "app.js declares function {name} twice");
    }
    assert!(seen.len() > 100, "the check found only {} functions", seen.len());
}
