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
        if node.path().is_ident("cfg_attr") {
            self.cfg_attr_lines.push(node.span().start().line);
        }
        syn::visit::visit_attribute(self, node);
    }
}
fn analyze(path: &Path, code: &str) -> Value {
    let parser = RustParser::new(code.as_bytes().to_vec(), path, None);
    let mut errors = vec![];
    if parser.get_root().has_error() {
        errors.push("rust-code-analysis grammar reported an error or missing node".to_owned());
    }
    let mut inventory = Inventory::default();
    match syn::parse_file(code) {
        Ok(file) => {
            if test_attrs(&file.attrs) {
                inventory.test_ranges.push((1, code.lines().count()));
            }
            inventory.visit_file(&file);
        }
        Err(e) => errors.push(format!(
            "syn syntax error at line {}: {e}",
            e.span().start().line
        )),
    }
    let metrics = rust_code_analysis::metrics(&parser, path);
    if metrics.is_none() {
        errors.push("analyzer returned no metric tree".into());
    }
    json!({"path":path,"errors":errors,"test_ranges":inventory.test_ranges,
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
}
