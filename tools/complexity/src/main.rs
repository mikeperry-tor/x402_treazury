//! Source-only metrics; independent of the application's dependencies and features.
use rust_code_analysis::{ParserTrait, RustParser};
use serde_json::{Value, json};
use std::{env, fs, path::Path};
use syn::{Meta, Token, parse::Parser, punctuated::Punctuated, spanned::Spanned, visit::Visit};

// Evaluate cfg with test=false and every other predicate unknown. Only a
// definitely false expression proves an item is unavailable outside test builds.
fn cfg_without_test(meta: &Meta) -> Option<bool> {
    match meta {
        Meta::Path(p) if p.is_ident("test") => Some(false),
        Meta::List(l) => {
            let children = Punctuated::<Meta, Token![,]>::parse_terminated
                .parse2(l.tokens.clone())
                .ok()?;
            let values: Vec<_> = children.iter().map(cfg_without_test).collect();
            if l.path.is_ident("all") {
                if values.contains(&Some(false)) {
                    Some(false)
                } else if values.iter().all(|v| *v == Some(true)) {
                    Some(true)
                } else {
                    None
                }
            } else if l.path.is_ident("any") {
                if values.contains(&Some(true)) {
                    Some(true)
                } else if values.iter().all(|v| *v == Some(false)) {
                    Some(false)
                } else {
                    None
                }
            } else if l.path.is_ident("not") && values.len() == 1 {
                values[0].map(|v| !v)
            } else {
                None
            }
        }
        _ => None,
    }
}
fn test_attrs(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().segments.last().is_some_and(|p| p.ident == "test")
            || (a.path().is_ident("cfg")
                && a.parse_args::<Meta>()
                    .ok()
                    .is_some_and(|m| cfg_without_test(&m) == Some(false)))
    })
}

// Lint levels cannot change item inclusion or test classification, regardless of
// whether their cfg predicate is true. Other conditional attributes still need
// review, including mixed lists and nested cfg_attr.
fn conditional_lints_only(attr: &syn::Attribute) -> bool {
    let Ok(args) = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) else {
        return false;
    };
    args.len() >= 2
        && args.iter().skip(1).all(|meta| {
            matches!(meta, Meta::List(list) if ["allow", "warn", "deny", "forbid", "expect"]
                .iter().any(|name| list.path.is_ident(name)))
        })
}

#[derive(Default)]
struct Inventory {
    test_ranges: Vec<(usize, usize)>,
    macro_lines: Vec<usize>,
    cfg_attr_lines: Vec<usize>,
    test_attribute_lines: Vec<usize>,
}
impl Inventory {
    fn attrs(&mut self, attrs: &[syn::Attribute], span: proc_macro2::Span) {
        if test_attrs(attrs) {
            self.test_ranges.push((span.start().line, span.end().line));
        }
    }
}
impl<'ast> Visit<'ast> for Inventory {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        let attrs = match item {
            syn::Item::Fn(v) => &v.attrs,
            syn::Item::Mod(v) => &v.attrs,
            syn::Item::Impl(v) => &v.attrs,
            syn::Item::Trait(v) => &v.attrs,
            syn::Item::Const(v) => &v.attrs,
            syn::Item::Static(v) => &v.attrs,
            _ => {
                syn::visit::visit_item(self, item);
                return;
            }
        };
        self.attrs(attrs, item.span());
        syn::visit::visit_item(self, item);
    }
    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        self.attrs(&node.attrs, node.span());
        syn::visit::visit_impl_item_fn(self, node);
    }
    fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
        self.attrs(&node.attrs, node.span());
        syn::visit::visit_trait_item_fn(self, node);
    }
    fn visit_expr_block(&mut self, node: &'ast syn::ExprBlock) {
        self.attrs(&node.attrs, node.span());
        syn::visit::visit_expr_block(self, node);
    }
    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        self.macro_lines.push(node.span().start().line);
        syn::visit::visit_macro(self, node);
    }
    fn visit_attribute(&mut self, node: &'ast syn::Attribute) {
        if test_attrs(std::slice::from_ref(node)) {
            self.test_attribute_lines.push(node.span().start().line);
        }
        if node.path().is_ident("cfg_attr") && !conditional_lints_only(node) {
            self.cfg_attr_lines.push(node.span().start().line);
        }
        syn::visit::visit_attribute(self, node);
    }
}
// The pinned grammar predates C string literals. Replace only their token prefix
// with whitespace for metrics, retaining every byte offset and newline. Original
// Rust is still checked by syn; string contents cannot introduce control flow.
fn c_string_compatibility(code: &str) -> Result<(Vec<u8>, Vec<usize>), String> {
    fn visit(tokens: proc_macro2::TokenStream, bytes: &mut [u8], lines: &mut Vec<usize>) {
        for token in tokens {
            match token {
                proc_macro2::TokenTree::Group(group) => visit(group.stream(), bytes, lines),
                proc_macro2::TokenTree::Literal(literal)
                    if syn::parse_str::<syn::LitCStr>(&literal.to_string()).is_ok() =>
                {
                    let offset = literal.span().byte_range().start;
                    assert_eq!(bytes[offset], b'c');
                    bytes[offset] = b' ';
                    lines.push(literal.span().start().line);
                }
                _ => {}
            }
        }
    }
    let tokens = code
        .parse()
        .map_err(|e| format!("token syntax error: {e}"))?;
    let mut bytes = code.as_bytes().to_vec();
    let mut lines = Vec::new();
    visit(tokens, &mut bytes, &mut lines);
    Ok((bytes, lines))
}

fn analyze(path: &Path, code: &str) -> Value {
    let mut errors = vec![];
    let mut metric_code = code.as_bytes().to_vec();
    let mut c_string_lines = Vec::new();
    let mut inventory = Inventory::default();
    match syn::parse_file(code) {
        Ok(file) => {
            if test_attrs(&file.attrs) {
                inventory.test_ranges.push((1, code.lines().count()));
            }
            inventory.visit_file(&file);
            match c_string_compatibility(code) {
                Ok((bytes, lines)) => {
                    metric_code = bytes;
                    c_string_lines = lines;
                }
                Err(error) => errors.push(error),
            }
        }
        Err(e) => errors.push(format!(
            "syn syntax error at line {}: {e}",
            e.span().start().line
        )),
    }
    let parser = RustParser::new(metric_code, path, None);
    if parser.get_root().has_error() {
        errors.push("rust-code-analysis grammar reported an error or missing node".to_owned());
    }
    let metrics = rust_code_analysis::metrics(&parser, path);
    if metrics.is_none() {
        errors.push("analyzer returned no metric tree".into());
    }
    json!({"path":path,"errors":errors,"c_string_lines":c_string_lines,"test_ranges":inventory.test_ranges,
        "macro_lines":inventory.macro_lines,"test_attribute_lines":inventory.test_attribute_lines,"cfg_attr_lines":inventory.cfg_attr_lines,"metrics":metrics})
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let paths: Vec<_> = env::args().skip(1).collect();
    if paths.is_empty() {
        return Err("usage: treazury-complexity FILE.rs [...]".into());
    }
    let files: Vec<_> = paths
        .iter()
        .map(|p| Ok(analyze(Path::new(p), &fs::read_to_string(p)?)))
        .collect::<Result<_, std::io::Error>>()?;
    println!(
        "{}",
        serde_json::to_string(
            &json!({"analyzer":"rust-code-analysis","version":"0.0.25","files":files})
        )?
    );
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("complexity analysis failed: {e}");
        std::process::exit(1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn functions(v: &Value, out: &mut Vec<Value>) {
        if v["kind"] == "function" {
            out.push(v.clone());
        }
        for child in v["spaces"].as_array().unwrap() {
            functions(child, out);
        }
    }
    fn score(code: &str) -> (f64, f64) {
        let v = analyze(Path::new("probe.rs"), code);
        assert_eq!(v["errors"], json!([]));
        let mut f = vec![];
        functions(&v["metrics"], &mut f);
        (
            f[0]["metrics"]["cyclomatic"]["sum"].as_f64().unwrap(),
            f[0]["metrics"]["cognitive"]["sum"].as_f64().unwrap(),
        )
    }
    #[test]
    fn branch_and_nesting_scores_are_directional() {
        let plain = score("fn f() { work(); }");
        let flat = score("fn f(a:bool,b:bool) {if a {work();} if b {work();}}");
        let nested = score("fn f(a:bool,b:bool) {if a {if b {work();}}}");
        assert_eq!(plain, (1.0, 0.0));
        assert_eq!(flat.0, nested.0);
        assert!(nested.1 > flat.1 && flat.1 > plain.1);
    }
    #[test]
    fn modern_syntax_closures_match_and_macros_are_visible() {
        let v = analyze(
            Path::new("probe.rs"),
            "async fn f(x: Option<i32>) -> Result<(), ()> { if let Some(v)=x && v>0 { run().await?; } let f=|v| match v {Some(x) if x>0=>1,_=>0}; ensure!(x.is_some()); Ok(()) }",
        );
        assert_eq!(v["errors"], json!([]));
        assert_eq!(v["macro_lines"], json!([1]));
        let mut f = vec![];
        functions(&v["metrics"], &mut f);
        assert!(f.len() >= 2);
        assert!(f.iter().any(|v| v["name"] == "<anonymous>"));
    }
    #[test]
    fn malformed_source_cannot_be_a_trusted_low_score() {
        for code in ["fn f( {", "fn f() { if true { foo(); }"] {
            assert!(
                !analyze(Path::new("bad.rs"), code)["errors"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn c_strings_preserve_scores_locations_and_literal_contents() {
        let code = "// c\"untouched\"\nfn f(a: bool) { let _ = \"é\"; let _ = c\"if else\";\nlet _ = cr#\"match \\\"\nwhile\"#; call!(c\"macro\"); if a { work(); } }";
        let (bytes, lines) = c_string_compatibility(code).unwrap();
        let normalized = String::from_utf8(bytes).unwrap();
        assert_eq!(normalized.len(), code.len());
        assert_eq!(lines, vec![2, 3, 4]);
        assert!(normalized.starts_with("// c\"untouched\""));
        assert_eq!(score(code), score(&normalized));
        let original = analyze(Path::new("probe.rs"), code);
        let compatible = analyze(Path::new("probe.rs"), &normalized);
        assert_eq!(original["metrics"], compatible["metrics"]);
        for bad in [
            "fn f() { let _ = c\"unterminated; }",
            "fn f( { let _ = c\"ok\"; }",
        ] {
            assert!(
                !analyze(Path::new("bad.rs"), bad)["errors"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
    }
    #[test]
    fn cfg_test_and_test_attributes_exclude_only_test_only_items() {
        let code = "#[cfg(all(unix,test))]\nmod checks { fn helper() {} }\n#[cfg(any(test,feature=\"live\"))]\nfn production() {}\n#[tokio::test]\nasync fn check() {}\n#[cfg(not(test))]\nfn normal() {}";
        let v = analyze(Path::new("probe.rs"), code);
        assert_eq!(v["errors"], json!([]));
        let ranges: Vec<(usize, usize)> = serde_json::from_value(v["test_ranges"].clone()).unwrap();
        assert_eq!(ranges, vec![(1, 2), (5, 6)]);
    }
    #[test]
    fn inline_test_blocks_and_unknown_cfg_attr_are_visible() {
        let v = analyze(
            Path::new("probe.rs"),
            "fn production() {\n#[cfg(test)]\n{ let check = || true; }\n}\n#[cfg_attr(feature=\"x\",test)]\nfn conditional() {}",
        );
        assert_eq!(v["errors"], json!([]));
        assert_eq!(v["test_ranges"], json!([[2, 3]]));
        assert_eq!(v["test_attribute_lines"], json!([2]));
        assert_eq!(v["cfg_attr_lines"], json!([5]));
    }

    #[test]
    fn conditional_lint_levels_preserve_metrics_and_classification() {
        for lint in ["allow", "warn", "deny", "forbid", "expect"] {
            let code = format!(
                "#[cfg_attr(not(feature=\"zcash\"), {lint}(dead_code), allow(clippy::all))]\nfn f(a: bool) {{ if a {{ work(); }} }}"
            );
            let v = analyze(Path::new("probe.rs"), &code);
            assert_eq!(v["errors"], json!([]));
            assert_eq!(v["cfg_attr_lines"], json!([]));
            assert_eq!(v["test_ranges"], json!([]));
            assert_eq!(score(&code), score("fn f(a: bool) { if a { work(); } }"));
        }
        let wallet = analyze(
            Path::new("wallet.rs"),
            "#[cfg_attr(not(feature=\"zcash\"), allow(dead_code))]\nenum Command { Recover }",
        );
        assert_eq!(wallet["errors"], json!([]));
        assert_eq!(wallet["cfg_attr_lines"], json!([]));
    }

    #[test]
    fn conditional_nonlint_attributes_still_require_review() {
        for args in [
            "feature=\"x\", allow(dead_code), test",
            "feature=\"x\", cfg(test)",
            "feature=\"x\", derive(Debug)",
            "feature=\"x\", custom::allow(dead_code)",
            "feature=\"x\", cfg_attr(test, allow(dead_code))",
            "feature=\"x\"",
        ] {
            let v = analyze(
                Path::new("probe.rs"),
                &format!("#[cfg_attr({args})]\nfn f() {{}}"),
            );
            assert_eq!(v["cfg_attr_lines"], json!([1]), "{args}");
        }
    }
}
