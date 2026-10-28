//! Explicit JSON field omissions; array traversal never removes records.
use serde_json::Value;

pub(super) fn apply(doc: &mut Value, paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .filter_map(|path| {
            let segments: Vec<_> = path
                .split('/')
                .skip(1)
                .map(|segment| segment.replace("~1", "/").replace("~0", "~"))
                .collect();
            let removed = omit_fields(doc, &segments);
            (removed > 0).then(|| {
                format!(
                    "JSON projection omitted {removed} field(s) matching {path:?}; those fields are absent from the returned JSON."
                )
            })
        })
        .collect()
}

fn omit_fields(value: &mut Value, segments: &[String]) -> usize {
    let Some((segment, rest)) = segments.split_first() else {
        return 0;
    };
    match value {
        Value::Object(fields) if rest.is_empty() => usize::from(fields.remove(segment).is_some()),
        Value::Object(fields) => fields
            .get_mut(segment)
            .map_or(0, |value| omit_fields(value, rest)),
        Value::Array(items) if segment == "*" => {
            items.iter_mut().map(|value| omit_fields(value, rest)).sum()
        }
        Value::Array(items) => segment
            .parse::<usize>()
            .ok()
            .filter(|index| index.to_string() == *segment)
            .and_then(|index| items.get_mut(index))
            .map_or(0, |value| omit_fields(value, rest)),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nested_wildcards_and_escaped_members_preserve_records_and_metadata() {
        let mut doc = json!({"results":[
            {"id":"a", "a/b":{"~figures":[1,2]}, "groups":[{"figures":[3]}, {"keep":4}]},
            {"id":"b", "a/b":{"~figures":null}, "groups":[]},
            {"id":"c"}, null, 5
        ], "page":2});
        let notices = apply(
            &mut doc,
            &[
                "/results/*/a~1b/~0figures".into(),
                "/results/*/groups/*/figures".into(),
                "/missing/figures".into(),
            ],
        );
        assert_eq!(
            doc,
            json!({"results":[
            {"id":"a", "a/b":{}, "groups":[{}, {"keep":4}]},
            {"id":"b", "a/b":{}, "groups":[]},
            {"id":"c"}, null, 5
        ], "page":2})
        );
        assert_eq!(notices.len(), 2);
        assert!(notices[0].contains("omitted 2 field(s)"));
        assert!(notices[1].contains("omitted 1 field(s)"));
    }

    #[test]
    fn array_indices_never_delete_elements_and_object_stars_are_literal() {
        let mut doc = json!({"results":[{"figures":[1]}, {"figures":[2]}], "*":{"x":3}, "":4});
        let notices = apply(
            &mut doc,
            &[
                "/results/0".into(),
                "/results/*".into(),
                "/results/01/figures".into(),
                "/results/+1/figures".into(),
                "/results/1/figures".into(),
                "/*/x".into(),
                "/".into(),
            ],
        );
        assert_eq!(doc, json!({"results":[{"figures":[1]}, {}], "*":{}}));
        assert_eq!(notices.len(), 3);
    }

    #[test]
    fn repeated_and_missing_paths_do_not_claim_omissions() {
        let mut doc = json!({"results":[{"figures":[]},{}]});
        let paths = vec!["/results/*/figures".into(), "/results/*/figures".into()];
        assert_eq!(apply(&mut doc, &paths).len(), 1);
        let unchanged = doc.clone();
        assert!(apply(&mut doc, &paths).is_empty());
        assert_eq!(doc, unchanged);
    }
}
