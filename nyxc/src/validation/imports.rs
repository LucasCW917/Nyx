// Stage 5 — Import Cycle Detector
// Reads the %import list from CompileConfig, recursively loads each imported
// module's own %make config to discover their imports, builds a directed graph,
// and runs DFS to detect cycles before type inference can be poisoned.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use crate::frontend::make_pass::{CompileConfig, run_make_pass};
use crate::frontend::lexer::lex;
use crate::frontend::parser::Program;
use super::{ValidationError, ValidationWarning};

pub fn check(
    program: Program,
    config: &CompileConfig,
) -> Result<(Program, Vec<ValidationWarning>), ValidationError> {
    if config.imports.is_empty() {
        return Ok((program, vec![]));
    }

    // Determine the base search path
    let base = config.look_for_path
        .as_deref()
        .unwrap_or(".");

    // Build the full import graph starting from the root module
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();
    let mut visited: HashSet<String> = HashSet::new();

    for module in &config.imports {
        build_graph(module, base, &mut graph, &mut visited)
            .map_err(|e| ValidationError::ImportCycleError {
                message: e,
                chain: vec![],
            })?;
    }

    // Run DFS cycle detection
    let mut stack: Vec<String> = Vec::new();
    let mut in_stack: HashSet<String> = HashSet::new();
    let mut fully_visited: HashSet<String> = HashSet::new();

    for module in graph.keys().cloned().collect::<Vec<_>>() {
        if !fully_visited.contains(&module) {
            detect_cycle(
                &module,
                &graph,
                &mut stack,
                &mut in_stack,
                &mut fully_visited,
            )?;
        }
    }

    Ok((program, vec![]))
}

/// Recursively load a module's %make config and add its imports to the graph.
fn build_graph(
    module: &str,
    base: &str,
    graph: &mut HashMap<String, Vec<String>>,
    visited: &mut HashSet<String>,
) -> Result<(), String> {
    if visited.contains(module) {
        return Ok(());
    }
    visited.insert(module.to_string());

    let path = resolve_module_path(module, base);

    // If the file doesn't exist, record it as having no imports
    // (the import resolver will report the missing file later)
    if !path.exists() {
        graph.insert(module.to_string(), vec![]);
        return Ok(());
    }

    let source = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read '{}': {}", path.display(), e))?;

    let tokens = lex(&source)
        .map_err(|e| format!("Lex error in '{}': {}", path.display(), e))?;

    let sub_config = run_make_pass(&tokens)
        .map_err(|e| format!("%%make error in '{}': {}", path.display(), e))?;

    let deps: Vec<String> = sub_config.imports.clone();
    graph.insert(module.to_string(), deps.clone());

    for dep in &deps {
        build_graph(dep, base, graph, visited)?;
    }

    Ok(())
}

/// DFS cycle detection. Returns Err with the cycle chain if a cycle is found.
fn detect_cycle(
    module: &str,
    graph: &HashMap<String, Vec<String>>,
    stack: &mut Vec<String>,
    in_stack: &mut HashSet<String>,
    fully_visited: &mut HashSet<String>,
) -> Result<(), ValidationError> {
    stack.push(module.to_string());
    in_stack.insert(module.to_string());

    if let Some(deps) = graph.get(module) {
        for dep in deps {
            if in_stack.contains(dep) {
                // Found a cycle — build the chain from the stack
                let cycle_start = stack.iter().position(|s| s == dep).unwrap_or(0);
                let mut chain: Vec<String> = stack[cycle_start..].to_vec();
                chain.push(dep.clone()); // close the loop
                return Err(ValidationError::ImportCycleError {
                    message: format!(
                        "Circular import detected: {} imports itself transitively",
                        dep
                    ),
                    chain,
                });
            }
            if !fully_visited.contains(dep) {
                detect_cycle(dep, graph, stack, in_stack, fully_visited)?;
            }
        }
    }

    stack.pop();
    in_stack.remove(module);
    fully_visited.insert(module.to_string());
    Ok(())
}

/// Resolve a module name to a file path.
/// "math" → "./math.nyx" (or look_for_path/math.nyx)
fn resolve_module_path(module: &str, base: &str) -> PathBuf {
    let filename = format!("{}.nyx", module);
    Path::new(base).join(filename)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_cycle_in_empty_graph() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["b".to_string()]);
        graph.insert("b".to_string(), vec![]);

        let mut stack = Vec::new();
        let mut in_stack = HashSet::new();
        let mut fully_visited = HashSet::new();

        assert!(detect_cycle("a", &graph, &mut stack, &mut in_stack, &mut fully_visited).is_ok());
    }

    #[test]
    fn test_direct_cycle_detected() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["b".to_string()]);
        graph.insert("b".to_string(), vec!["a".to_string()]);

        let mut stack = Vec::new();
        let mut in_stack = HashSet::new();
        let mut fully_visited = HashSet::new();

        let result = detect_cycle("a", &graph, &mut stack, &mut in_stack, &mut fully_visited);
        assert!(result.is_err());
        if let Err(ValidationError::ImportCycleError { chain, .. }) = result {
            assert!(chain.contains(&"a".to_string()));
            assert!(chain.contains(&"b".to_string()));
        }
    }

    #[test]
    fn test_transitive_cycle_detected() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["b".to_string()]);
        graph.insert("b".to_string(), vec!["c".to_string()]);
        graph.insert("c".to_string(), vec!["a".to_string()]);

        let mut stack = Vec::new();
        let mut in_stack = HashSet::new();
        let mut fully_visited = HashSet::new();

        assert!(detect_cycle("a", &graph, &mut stack, &mut in_stack, &mut fully_visited).is_err());
    }

    #[test]
    fn test_resolve_module_path() {
        let path = resolve_module_path("math", ".");
        assert_eq!(path, Path::new("./math.nyx"));
    }

    #[test]
    fn test_resolve_module_path_custom_base() {
        let path = resolve_module_path("utils", "./libs");
        assert_eq!(path, Path::new("./libs/utils.nyx"));
    }
}