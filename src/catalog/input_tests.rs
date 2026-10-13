use super::*;

fn complete_input(op: &Value) -> Value {
    let input = operation_input(op).unwrap();
    json!({"schema":input.schema,"routes":input.routes,
        "body_names":input.body_names,"has_body":input.has_body})
}

#[test]
fn parameter_defaults_duplicates_and_skips_preserve_complete_input() {
    let op = json!({"params":[
        {"name":"duplicate","in":"path","required":true,"schema":{"type":"integer"}},
        {"name":"duplicate","in":"cookie","required":false,"description":"last","schema":null},
        {"name":"header","in":"header","required":true,"schema":{"type":"number"}},
        {"in":"path","required":true}, {"name":7,"required":true},
        {"name":"bare","required":"true","schema":[]},
        {"name":"empty","description":"","schema":false},
        {"name":"schema_wins","description":"parameter","schema":{"description":"","type":null}},
        {"name":"path","in":"path","required":true,"schema":{"minimum":2,"exclusiveMinimum":true}},
        {"name":"","required":true}
    ]});
    assert_eq!(
        complete_input(&op),
        json!({
            "schema":{"type":"object","properties":{
                "duplicate":{"type":"string","description":"last"},
                "bare":{"type":"string"},"empty":{"type":"string"},
                "schema_wins":{"description":"","type":null},
                "path":{"type":"string","minimum":2,"exclusiveMinimum":2},
                "":{"type":"string"}},"required":["","duplicate","path"]},
            "routes":{"duplicate":"query","bare":"query","empty":"query","schema_wins":"query","path":"path","":"query"},
            "body_names":{},"has_body":false
        })
    );
    // Later optional duplicates replace schema and route, but do not remove an
    // earlier required name. Header duplicates do not replace anything.
    let op = json!({"params":[{"name":"x","in":"query","required":true},
        {"name":"x","in":"header","schema":{"type":"number"}}]});
    assert_eq!(
        complete_input(&op),
        json!({"schema":{"type":"object","properties":{"x":{"type":"string"}},"required":["x"]},"routes":{"x":"query"},"body_names":{},"has_body":false})
    );
}

#[test]
fn missing_null_empty_and_nonobject_bodies_preserve_presence() {
    for op in [
        json!({}),
        json!({"params":null,"body":null}),
        json!({"params":{}}),
    ] {
        assert_eq!(
            complete_input(&op),
            json!({"schema":{"type":"object","properties":{}},"routes":{},"body_names":{},"has_body":false})
        );
    }
    for body in [
        json!({}),
        json!(false),
        json!(0),
        json!([]),
        json!({"schema":null}),
        json!({"schema":{"type":"string","required":["missing"]}}),
        json!({"schema":{"properties":[],"required":["missing"]}}),
    ] {
        assert_eq!(
            complete_input(&json!({"body":body})),
            json!({"schema":{"type":"object","properties":{}},"routes":{},"body_names":{},"has_body":true})
        );
    }
}

#[test]
fn body_properties_preserve_renaming_defaults_required_and_recursive_sanitization() {
    let op = json!({"params":[{"name":"id","in":"path","required":true}, {"name":"q","in":"query"}],
        "body":{"required":false,"schema":{"properties":{
            "id":{"type":"integer","maximum":9,"exclusiveMaximum":true},
            "q":false,"empty":{},"null":null,
            "nested":{"type":"array","items":{"type":"number","minimum":1,"exclusiveMinimum":false,"exclusiveMaximum":7}}
        },"required":["q","id","missing",7]}}});
    assert_eq!(
        complete_input(&op),
        json!({"schema":{"type":"object","properties":{
        "id":{"type":"string"},"q":{"type":"string"},
        "id_body":{"type":"integer","maximum":9,"exclusiveMaximum":9},
        "q_body":{"type":"string"},"empty":{},"null":{"type":"string"},
        "nested":{"type":"array","items":{"type":"number","minimum":1,"exclusiveMaximum":7}}
    },"required":["id","id_body","q_body"]},
    "routes":{"id":"path","q":"query","id_body":"body","q_body":"body","empty":"body","null":"body","nested":"body"},
    "body_names":{"id_body":"id","q_body":"q","empty":"empty","null":"null","nested":"nested"},"has_body":true})
    );
}

#[test]
fn occupied_body_names_reject_with_exact_first_collision() {
    for (op, expected) in [
        (
            json!({"params":[{"name":"q"},{"name":"q_body"}],"body":{"schema":{"properties":{"q":{}}}}}),
            "q",
        ),
        (
            json!({"params":[{"name":"a"},{"name":"a_body"},{"name":"z"},{"name":"z_body"}],"body":{"schema":{"properties":{"z":{},"a":{}}}}}),
            "a",
        ),
        (
            json!({"params":[{"name":"q"},{"name":"q_body_body"}],"body":{"schema":{"properties":{"q":{},"q_body":{}}}}}),
            "q_body",
        ),
    ] {
        let error = match operation_input(&op) {
            Ok(_) => panic!("collision was accepted"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            format!("unresolvable body/query collision: {expected}")
        );
    }
    // A collision with an already merged body property is renamed again when
    // that second renamed key is free; this is existing accepted behavior.
    assert_eq!(
        complete_input(
            &json!({"params":[{"name":"q"}],"body":{"schema":{"properties":{"q":{},"q_body":{}}}}})
        ),
        json!({"schema":{"type":"object","properties":{"q":{"type":"string"},"q_body":{},"q_body_body":{}}},"routes":{"q":"query","q_body":"body","q_body_body":"body"},"body_names":{"q_body":"q","q_body_body":"q_body"},"has_body":true})
    );
}

#[test]
fn presence_alternatives_translate_only_complete_supported_branches() {
    let base = json!({"params":[{"name":"ids","in":"query"}],"body":{"schema":{"properties":{"ids":{},"urls":{}}}}});
    let expected = json!({"schema":{"type":"object","properties":{"ids":{"type":"string"},"ids_body":{},"urls":{}}},"routes":{"ids":"query","ids_body":"body","urls":"body"},"body_names":{"ids_body":"ids","urls":"urls"},"has_body":true});
    for keyword in ["oneOf", "anyOf", "allOf"] {
        for (branches, translated) in [
            (
                json!([{"required":["ids"]},{"required":["urls","ids","ids"]}]),
                Some(
                    json!([{"required":["ids_body"]},{"required":["urls","ids_body","ids_body"]}]),
                ),
            ),
            (json!([]), Some(json!([]))),
            (json!([{"required":[]}]), Some(json!([{"required":[]}]))),
            (json!([{"required":["ids"]},{"required":["unknown"]}]), None),
            (json!([{"required":["ids"]},{"required":[7]}]), None),
            (json!([{"required":["ids"],"type":"object"}]), None),
            (json!([{}]), None),
            (json!([null]), None),
            (json!([{"required":"ids"}]), None),
            (json!({}), None),
        ] {
            let mut op = base.clone();
            op["body"]["schema"][keyword] = branches;
            let mut output = expected.clone();
            if let Some(translated) = translated {
                output["schema"][keyword] = translated;
            }
            assert_eq!(complete_input(&op), output, "{keyword}: {op}");
        }
    }
}
