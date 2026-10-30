//! Regression contract for a bounded, derived file-dependency graph.
use super::{build_graph, SourceDocument};
use serde_json::Value;
use sha2::Digest;

fn source(path: &str, content: &str) -> SourceDocument {
    SourceDocument {
        path: path.to_owned(),
        source_hash: format!("{:x}", sha2::Sha256::digest(content.as_bytes())),
        content: content.to_owned(),
    }
}

fn edge<'a>(graph: &'a Value) -> &'a Value {
    &graph["edges"].as_array().expect("edges array")[0]
}

#[test]
fn javascript_relative_imports_are_resolved_only_from_observed_files() {
    let docs = vec![
        source("src/main.ts", "import { load } from './utils';\n"),
        source("src/utils.ts", "export function load() {}\n"),
    ];
    let graph = build_graph(&docs, 64, None);
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(edge(&graph)["kind"], "import");
    assert_eq!(edge(&graph)["status"], "resolved");
    assert_eq!(edge(&graph)["from"], "src/main.ts");
    assert_eq!(edge(&graph)["to"], "src/utils.ts");
    assert_eq!(edge(&graph)["line"], 1);
    assert_eq!(edge(&graph)["evidence_hash"], docs[0].source_hash);
}

#[test]
fn ambiguous_relative_import_is_not_selected_arbitrarily() {
    let docs = vec![
        source("src/main.ts", "import './helpers';\n"),
        source("src/helpers.ts", "export const a = 1;\n"),
        source("src/helpers.js", "exports.a = 1;\n"),
    ];
    let graph = build_graph(&docs, 64, None);
    assert_eq!(edge(&graph)["status"], "ambiguous");
    assert!(edge(&graph)["to"].is_null());
    assert_eq!(edge(&graph)["candidate_count"], 2);
}

#[test]
fn missing_and_out_of_root_references_are_unresolved() {
    let docs = vec![source(
        "src/index.js",
        "import other from './missing.js';\nimport outside from '../../private';\n",
    )];
    let graph = build_graph(&docs, 64, None);
    assert_eq!(graph["edges"].as_array().unwrap().len(), 2);
    for relation in graph["edges"].as_array().unwrap() {
        assert_eq!(relation["status"], "unresolved");
        assert!(relation["to"].is_null());
    }
    assert!(!graph.to_string().contains("private/secret"));
}

#[test]
fn rust_module_paths_resolve_to_their_actual_file() {
    let docs = vec![
        source("src/lib.rs", "pub mod api;\n"),
        source("src/api/mod.rs", "pub fn list() {}\n"),
    ];
    let graph = build_graph(&docs, 64, None);
    assert_eq!(edge(&graph)["kind"], "module");
    assert_eq!(edge(&graph)["status"], "resolved");
    assert_eq!(edge(&graph)["to"], "src/api/mod.rs");
}

#[test]
fn ambiguous_rust_module_paths_are_never_assumed() {
    let docs = vec![
        source("src/lib.rs", "mod api;\n"),
        source("src/api.rs", "pub fn one() {}\n"),
        source("src/api/mod.rs", "pub fn two() {}\n"),
    ];
    let graph = build_graph(&docs, 64, None);
    assert_eq!(edge(&graph)["status"], "ambiguous");
}

#[test]
fn impact_follows_only_confirmed_inbound_dependencies() {
    let docs = vec![
        source("src/a.ts", "import './b';\n"),
        source("src/b.ts", "export const value = 1;\n"),
        source("src/c.ts", "import './a';\n"),
        source("src/d.ts", "import './missing';\n"),
    ];
    let graph = build_graph(&docs, 64, Some("src/b.ts"));
    assert_eq!(graph["impact"], serde_json::json!(["src/a.ts", "src/c.ts"]));
    assert_eq!(graph["impact_exhaustive"], false);
}

#[test]
fn edge_budget_reports_partial_projection() {
    let docs = vec![
        source("src/a.ts", "import './b';\nimport './c';\n"),
        source("src/b.ts", "export const b = 1;\n"),
        source("src/c.ts", "export const c = 1;\n"),
    ];
    let graph = build_graph(&docs, 1, None);
    assert_eq!(graph["edges"].as_array().unwrap().len(), 1);
    assert_eq!(graph["edges_complete"], false);
}

#[test]
fn no_claim_of_complete_language_or_semantic_coverage() {
    let graph = build_graph(&[source("app/main.py", "import requests\n")], 64, None);
    assert_eq!(graph["semantic_graph"], false);
    assert_eq!(graph["snapshot_atomic"], false);
    assert_eq!(graph["edges"].as_array().unwrap().len(), 0);
}

#[test]
fn dotted_typescript_stem_is_not_mistaken_for_a_known_extension() {
    let docs = vec![
        source("src/index.ts", "import './user.service';\n"),
        source("src/user.service.ts", "export const service = true;\n"),
    ];
    let graph = build_graph(&docs, 64, None);
    assert_eq!(edge(&graph)["status"], "resolved");
    assert_eq!(edge(&graph)["to"], "src/user.service.ts");
}

#[test]
fn generated_graph_json_respects_output_budget_without_broken_json() {
    let mut docs = Vec::new();
    for i in 0..32 {
        let name = format!("src/{}_{}.ts", "a".repeat(125), i);
        let mut body = String::new();
        for j in 0..32 {
            if i != j {
                body.push_str(&format!("import '../src/{}_{}';\n", "a".repeat(125), j));
            }
        }
        docs.push(source(&name, &body));
    }
    let graph = build_graph(&docs, 64, Some(&docs[0].path));
    let json_body = graph.to_string();
    assert!(
        json_body.len() <= 26_000,
        "unbounded graph result: {}",
        json_body.len()
    );
    let roundtrip: Value = serde_json::from_str(&json_body).unwrap();
    assert_eq!(roundtrip["edges_complete"], false);
    assert!(roundtrip["edges"].is_array());
    assert!(roundtrip["nodes"].is_array());
}

#[test]
fn truncated_graph_does_not_report_unprovable_reverse_impact() {
    let mut docs = Vec::new();
    for i in 0..32 {
        let stem = format!("{}_{}", "a".repeat(125), i);
        let mut imports = String::new();
        for j in 0..32 {
            if j != i {
                imports.push_str(&format!("import './{}_{}';\n", "a".repeat(125), j));
            }
        }
        docs.push(source(&format!("src/{stem}.ts"), &imports));
    }
    let graph = build_graph(&docs, 64, Some(&docs[0].path));
    assert_eq!(graph["edges_complete"], false);
    if graph["edge_truncated_reason"] == "max_graph_result_bytes" {
        assert_eq!(graph["impact"], serde_json::json!([]));
        assert_eq!(graph["impact_omitted_reason"], "bounded_edges");
    }
}

#[test]
fn python_relative_imports_resolve_module_file_and_package() {
    let docs = vec![
        source(
            "src/pkg/main.py",
            "from .helpers import do_thing\nfrom .subpackage import api\n",
        ),
        source("src/pkg/helpers.py", "def do_thing(): pass\n"),
        source("src/pkg/subpackage/__init__.py", "def api(): pass\n"),
    ];
    let graph = build_graph(&docs, 64, None);
    let edges = graph["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 2);
    assert_eq!(edges[0]["kind"], "python_import");
    assert_eq!(edges[0]["status"], "resolved");
    assert_eq!(edges[0]["to"], "src/pkg/helpers.py");
    assert_eq!(edges[1]["to"], "src/pkg/subpackage/__init__.py");
}

#[test]
fn python_parent_relative_import_does_not_escape_observed_root() {
    let docs = vec![
        source(
            "src/pkg/tasks/run.py",
            "from ..shared import helper\nfrom .....escape import secret\n",
        ),
        source("src/pkg/shared.py", "helper = 1\n"),
    ];
    let graph = build_graph(&docs, 64, None);
    let edges = graph["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 2);
    assert_eq!(edges[0]["status"], "resolved");
    assert_eq!(edges[0]["to"], "src/pkg/shared.py");
    assert_eq!(edges[1]["status"], "unresolved");
    assert!(edges[1]["to"].is_null());
}

#[test]
fn python_from_current_package_can_reference_multiple_imported_children() {
    let docs = vec![
        source("package/__init__.py", "from . import alpha, beta\n"),
        source("package/alpha.py", "alpha = 1\n"),
        source("package/beta.py", "beta = 2\n"),
    ];
    let graph = build_graph(&docs, 64, None);
    let edges = graph["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 2);
    assert_eq!(edges[0]["to"], "package/alpha.py");
    assert_eq!(edges[1]["to"], "package/beta.py");
}

#[test]
fn python_matching_module_file_and_package_is_ambiguous() {
    let docs = vec![
        source("src/main.py", "from .utils import value\n"),
        source("src/utils.py", "value = 1\n"),
        source("src/utils/__init__.py", "value = 2\n"),
    ];
    let graph = build_graph(&docs, 64, None);
    assert_eq!(edge(&graph)["status"], "ambiguous");
    assert_eq!(edge(&graph)["candidate_count"], 2);
}

#[test]
fn javascript_dynamic_literal_import_is_a_possible_dependency() {
    let docs = vec![
        source("app/route.ts", "const load = () => import('./view');\n"),
        source("app/view.tsx", "export function View() {}\n"),
    ];
    let graph = build_graph(&docs, 64, None);
    assert_eq!(edge(&graph)["kind"], "dynamic_import");
    assert_eq!(edge(&graph)["status"], "resolved");
    assert_eq!(edge(&graph)["to"], "app/view.tsx");
}

#[test]
fn explicit_json_and_stylesheet_imports_resolve_exact_observed_assets() {
    let docs = vec![
        source(
            "app/index.ts",
            "import data from './settings.json';\nimport './theme.css';\n",
        ),
        source("app/settings.json", "{\\\"healthy\\\":true}"),
        source("app/theme.css", "body { color: inherit; }"),
    ];
    let graph = build_graph(&docs, 64, None);
    let edges = graph["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 2);
    assert_eq!(edges[0]["status"], "resolved");
    assert_eq!(edges[0]["to"], "app/settings.json");
    assert_eq!(edges[1]["to"], "app/theme.css");
}

#[test]
fn graph_does_not_invent_edges_from_js_comments_or_string_literals() {
    let docs = vec![
        source(
            "src/main.ts",
            r#"
            const quoted = "require('./fake')";
            const inline = 1; // require('./fake')
            /* import('./fake') */
            /*
               import Ghost from './fake';
               require('./fake')
            */
            const real = require('./actual'); /* require('./fake') */
            const deferred = import('./lazy');
        "#,
        ),
        source("src/fake.ts", "export const notImported = 1;\n"),
        source("src/actual.ts", "export const actual = true;\n"),
        source("src/lazy.ts", "export const lazy = true;\n"),
    ];
    let result = build_graph(&docs, 64, None);
    let edges = result["edges"].as_array().unwrap();
    assert_eq!(
        edges.len(),
        2,
        "comments and strings are not dependency syntax"
    );
    assert_eq!(edges[0]["to"], "src/actual.ts");
    assert_eq!(edges[1]["to"], "src/lazy.ts");
    assert!(edges.iter().all(|e| e["specifier"] != "./fake"));
}

#[test]
fn graph_does_not_parse_rust_mod_declarations_inside_block_comments() {
    let docs = vec![
        source("src/lib.rs", "/*\nmod pretend;\n*/\nmod actual;\n"),
        source("src/pretend.rs", "pub const V: u8 = 1;\n"),
        source("src/actual.rs", "pub const V: u8 = 2;\n"),
    ];
    let result = build_graph(&docs, 64, None);
    let edges = result["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["to"], "src/actual.rs");
    assert_eq!(edges[0]["line"], 4);
}

#[test]
fn graph_does_not_treat_python_docstring_examples_as_imports() {
    let docs = vec![
        source(
            "pkg/main.py",
            "\"\"\"\nfrom .fake import f\n\"\"\"\nfrom .actual import a\n",
        ),
        source("pkg/fake.py", "f = 1\n"),
        source("pkg/actual.py", "a = 2\n"),
    ];
    let graph = build_graph(&docs, 64, None);
    let edges = graph["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["to"], "pkg/actual.py");
    assert_eq!(edges[0]["line"], 4);
}

#[test]
fn rust_path_attribute_remains_authoritative_across_intervening_comments() {
    let docs = vec![
        source(
            "src/lib.rs",
            "#[path = \"custom.rs\"]\n// documentation comment\nmod alternate;\nmod actual;\n",
        ),
        source("src/alternate.rs", "pub const BAD: u8 = 1;\n"),
        source("src/custom.rs", "pub const REAL: u8 = 2;\n"),
        source("src/actual.rs", "pub const GOOD: u8 = 3;\n"),
    ];
    let graph = build_graph(&docs, 64, None);
    let edges = graph["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["to"], "src/actual.rs");
}

#[test]
fn nested_inline_rust_modules_are_not_resolved_against_root_files() {
    let docs = vec![
        source("src/lib.rs", "mod nested {\n  mod child;\n}\nmod actual;\n"),
        source("src/child.rs", "pub const FALSE_TARGET: u8 = 1;\n"),
        source("src/actual.rs", "pub const REAL: u8 = 2;\n"),
    ];
    let graph = build_graph(&docs, 64, None);
    let edges = graph["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["to"], "src/actual.rs");
}

#[test]
fn rust_raw_multiline_strings_never_create_source_module_edges() {
    let documents = vec![
        source(
            "src/lib.rs",
            r####"
const MESSAGE: &str = r###"
mod fake_one;
fn demonstration() { mod fake_two; }
"###;
mod genuine;
"####,
        ),
        source("src/fake_one.rs", "pub const FALSE: u8 = 1;"),
        source("src/fake_two.rs", "pub const FALSE: u8 = 2;"),
        source("src/genuine.rs", "pub const TRUE: u8 = 3;"),
    ];
    let graph = build_graph(&documents, 64, None);
    let edges = graph["edges"].as_array().unwrap();
    assert_eq!(
        edges.len(),
        1,
        "module-looking raw string bytes must not become graph edges"
    );
    assert_eq!(edges[0]["to"], "src/genuine.rs");
}

#[test]
fn rust_multiline_ordinary_string_and_raw_byte_string_do_not_create_edges() {
    let documents = vec![
        source(
            "src/lib.rs",
            r####"
const NORMAL: &str = "one
mod fake_one;
three";
const BINARY: &[u8] = br##"
mod fake_two;
"##;
mod genuine;
"####,
        ),
        source("src/fake_one.rs", "pub const FALSE: u8 = 1;"),
        source("src/fake_two.rs", "pub const FALSE: u8 = 2;"),
        source("src/genuine.rs", "pub const TRUE: u8 = 3;"),
    ];
    let graph = build_graph(&documents, 64, None);
    let edges = graph["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["to"], "src/genuine.rs");
}

#[test]
fn rust_module_path_attribute_inside_raw_string_is_not_a_real_attribute() {
    let documents = vec![
        source(
            "src/lib.rs",
            r####"
const EXAMPLE: &str = r##"
#[path = "fake.rs"]
"##;
mod genuine;
"####,
        ),
        source("src/genuine.rs", "pub const YES: u8 = 1;"),
        source("src/fake.rs", "pub const NO: u8 = 1;"),
    ];
    let graph = build_graph(&documents, 64, None);
    let edges = graph["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["to"], "src/genuine.rs");
}

#[test]
fn js_from_import_must_be_outside_trailing_comments() {
    let docs = vec![
        source(
            "src/main.ts",
            r#"
            const value = 1;
            export { value }; // from './phantom'
            export { value }; /* from './phantom' */
            export { value } from './real';
        "#,
        ),
        source("src/phantom.ts", "export const falseEdge = true;\n"),
        source("src/real.ts", "export const value = 2;\n"),
    ];
    let graph = build_graph(&docs, 64, None);
    let edges = graph["edges"].as_array().unwrap();
    assert_eq!(
        edges.len(),
        1,
        "commented from must not create a dependency"
    );
    assert_eq!(edges[0]["to"], "src/real.ts");
}

#[test]
fn js_real_from_clause_accepts_module_paths_containing_the_word_from() {
    let docs = vec![
        source("src/index.ts", "export { value } from './from-utils';\n"),
        source("src/from-utils.ts", "export const value = 1;\n"),
    ];
    let graph = build_graph(&docs, 64, None);
    let edges = graph["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["status"], "resolved");
    assert_eq!(edges[0]["to"], "src/from-utils.ts");
}
