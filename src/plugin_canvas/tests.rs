use super::*;

#[test]
fn sdk_spike_canvas_pet_validates_mood_bindings_and_budget() {
    use serde_json::json;
    let runner = json!({"pet": {
        "id": "runner", "at": [{"bind": "worked", "map": [20, 220]}, 24],
        "size": 28, "mood": "praised", "motion": "run"
    }});
    let parse = |nodes| {
        Canvas::parse(json!({
            "height":80, "nodes":nodes, "binds":{"worked":{"progress":[100, 200]}}
        }))
    };
    let canvas = parse(vec![runner.clone()]).unwrap();
    assert_eq!(canvas.nodes[0].id.as_deref(), Some("runner"));
    let (evaluated, _) = canvas.evaluate(chrono::DateTime::from_timestamp(150, 0).unwrap(), 1.0);
    let Kind::Pet {
        at, mood, motion, ..
    } = &evaluated.nodes[0].kind
    else {
        panic!("缺少岛仔节点")
    };
    assert_eq!(at[0].initial(), 120.0);
    assert_eq!(
        *mood,
        PetMood::Expression(crate::pet_expressions::Expression::Praised)
    );
    assert_eq!(*motion, PetMotion::Run);
    assert!(parse(vec![runner.clone(), runner.clone()])
        .unwrap_err()
        .contains("一只"));
    let mut too_many = vec![runner];
    too_many
        .extend((0..8).map(|_| json!({"circle":[5,5,2],"loop":{"kind":"pulse","period_ms":800}})));
    assert!(parse(too_many).unwrap_err().contains("循环"));
    assert!(parse(vec![json!({"pet":{"at":[0,0],"size":20,"mood":"unknown"}})]).is_err());
}
use serde_json::{json, Value};

fn with_nodes(nodes: Value) -> Result<Canvas, String> {
    Canvas::parse(json!({"height":196,"nodes":nodes}))
}

#[test]
fn sdk_spike_canvas_parses_all_primitives() {
    let canvas = with_nodes(json!([
        {"rect":[0,0,150,196],"r":[10,0,0,6],"fill":"$card","shadow":"lg"},
        {"circle":[16,12,4],"fill":{"radial":[[16,12],4],"stops":[[0,"#FFE58A"],[1,"#FF000000"]]}},
        {"arc":{"c":[16,22],"r":14,"start":-90,"sweep":210},"stroke":"#FFFFFF","width":2,"glow":true},
        {"line":[2,24,80,24],"stroke":"$dim","cap":"round","dash":[2,3]},
        {"points":[[0,32],[12,30],[28,35]],"area":42,"fill":{"linear":[[0,0],[0,42]],"stops":[[0,"$accent"],[1,"#00000000"]]}},
        {"path":"M1 1 l3 0 h2 V5 q1 2 0 3 c1 1 2 2 3 3 Z","stroke":"$accent"},
        {"text":"距元旦","at":[4,56],"size":16,"weight":700,"align":"center","max_width":32,"lines":2,"font":"快乐体","color":"$text"},
        {"image":"assets/cover.SVG","rect":[0,60,150,120],"fit":"cover"},
        {"group":[{"rect":[0,0,12,10],"r":4}],"clip":[0,0,20,20],"translate":[30,40],"rotate":15,"scale":1.2}
    ])).unwrap();
    assert_eq!(canvas.height, 196.0);
    assert!(matches!(&canvas.nodes[0].kind, Kind::Rect { r, .. } if r[3].initial() == 6.0));
    assert!(matches!(&canvas.nodes[1].kind, Kind::Circle(..)));
    assert!(matches!(&canvas.nodes[2].kind, Kind::Arc { .. }));
    assert!(matches!(&canvas.nodes[3].kind, Kind::Line(..)));
    assert!(matches!(&canvas.nodes[4].kind, Kind::Polyline { .. }));
    assert!(
        matches!(&canvas.nodes[5].kind, Kind::Path(commands) if commands == &[
            PathCommand::Move([1.0,1.0]),
            PathCommand::Line([4.0,1.0]),
            PathCommand::Line([6.0,1.0]),
            PathCommand::Line([6.0,5.0]),
            PathCommand::Quad([7.0,7.0],[6.0,8.0]),
            PathCommand::Cubic([7.0,9.0],[8.0,10.0],[9.0,11.0]),
            PathCommand::Close,
        ])
    );
    assert!(matches!(&canvas.nodes[6].kind, Kind::Text { .. }));
    assert!(matches!(&canvas.nodes[7].kind, Kind::Image { .. }));
    assert!(matches!(&canvas.nodes[8].kind, Kind::Group { .. }));
    assert!(canvas
        .validate_images(|path| path == "assets/cover.SVG")
        .is_ok());
    assert!(canvas
        .validate_images(|_| false)
        .unwrap_err()
        .contains("cover.SVG"));
}

#[test]
fn sdk_spike_canvas_limits_nodes_depth_path_commands_and_text() {
    let node = json!({"rect":[0,0,1,1]});
    assert!(with_nodes(json!(vec![node.clone(); 1500])).is_ok());
    assert!(with_nodes(json!(vec![node.clone(); 1501]))
        .unwrap_err()
        .contains("1500"));

    let mut group = node;
    for _ in 0..7 {
        group = json!({"group":[group]});
    }
    assert!(with_nodes(json!([group.clone()])).is_ok());
    assert!(with_nodes(json!([{"group":[group]}]))
        .unwrap_err()
        .contains("8 层"));

    let path = format!("M0 0{}", " L1 1".repeat(1999));
    assert!(with_nodes(json!([{"path":path},{"path":path}])).is_ok());
    assert!(
        with_nodes(json!([{"path":path},{"path":path},{"path":"M1 1"}]))
            .unwrap_err()
            .contains("4000")
    );

    let text = "岛".repeat(1000);
    assert!(with_nodes(
        json!([{"text":text,"at":[0,0],"size":6},{"text":text,"at":[0,0],"size":96}])
    )
    .is_ok());
    assert!(with_nodes(
        json!([{"text":text,"at":[0,0],"size":6},{"text":text,"at":[0,0],"size":6},
        {"text":"仔","at":[0,0],"size":6}])
    )
    .unwrap_err()
    .contains("2000"));
}

#[test]
fn sdk_spike_canvas_rejects_invalid_values_and_paths() {
    for height in [39.99, 360.01] {
        assert!(Canvas::parse(json!({"height":height,"nodes":[]})).is_err());
    }
    for height in [40, 360] {
        assert!(Canvas::parse(json!({"height":height,"nodes":[]})).is_ok());
    }
    for node in [
        json!({"rect":[2001,0,40,40]}),
        json!({"rect":[0,0,-1,40]}),
        json!({"rect":[0,0,40,40],"opacity":1.01}),
        json!({"rect":[0,0,40,40],"fill":"$unknown"}),
        json!({"rect":[0,0,40,40],"fill":"#xy0000"}),
        json!({"rect":[0,0,40,40],"fill":{"linear":[[0,0],[9,9]],"stops":[[1,"$dim"],[0,"$text"]]}}),
        json!({"circle":[0,0,0]}),
        json!({"line":[0,0,0,0],"dash":[0,0]}),
        json!({"text":"岛","at":[0,0],"size":5}),
        json!({"text":"岛","at":[0,0],"size":97}),
        json!({"text":"岛","at":[0,0],"size":1e100}),
        json!({"text":"岛","at":[0,0],"size":12,"lines":0}),
        json!({"path":"M0 0 T10 10"}),
        json!({"path":"M1999 0 l2 0"}),
        json!({"path":"M0 0 Q3000 0 2 3"}),
        json!({"path":"M0 NaN"}),
        json!({"rect":[0,0,1,1],"circle":[0,0,1]}),
        json!({"rect":[0,0,1,1],"unknown":true}),
        json!({"rect":[0,0,1,1],"fill":{"bind":"absent","pick":["$text","$dim"]}}),
    ] {
        assert!(with_nodes(json!([node])).is_err(), "{node}");
    }
    for image in [
        "../outside.png",
        "a/../outside.svg",
        "a..b.svg",
        "D:/outside.png",
        "/outside.svg",
        "\\\\server\\outside.svg",
        "assets\\cover.svg",
        "assets//cover.svg",
        "data:image/png",
        "assets/not-an-image.txt",
    ] {
        assert!(!valid_image_path(image), "{image}");
        assert!(
            with_nodes(json!([{"image":image,"rect":[0,0,20,20]}])).is_err(),
            "{image}"
        );
    }
}

#[test]
fn sdk_spike_canvas_preserves_live_declarations() {
    let canvas = Canvas::parse(json!({
        "height":40,
        "binds":{
            "money":{"counter":{"start_value":0,"per_second":0.0167,"start_at":100,"decimals":2}},
            "work":{"progress":[100,200]},
            "left":{"countdown":{"target_at":200,"format":"HH:mm:ss"}},
            "clock":{"clock_angle":{"zone":"America/New_York","hand":"hour"}},
            "now":{"now":"HH:mm","zone":"Asia/Shanghai"},
            "day":{"daylight":{"zone":"America/New_York","hours":[6,18]}}
        },
        "nodes":[
            {"group":[{"text":"{money}","at":[0,12],"size":12,"roll":true,"hit":{"action":"change-tab"}}],
                "clip":[0,0,20,20],"scroll":{"axis":"y","content":50}},
            {"rect":[0,10,{"bind":"work","map":[0,15]},8],"id":"bar","animate":{"ms":220,"ease":"spring"},
                "loop":{"kind":"pulse","period_ms":800},"burst":"confetti",
                "fill":{"bind":"day","pick":["$dim","$accent"]},"hover":{"opacity":1,"lift":2,"fill":"$card"}}
        ]
    })).unwrap();
    assert_eq!(canvas.binds.len(), 6);
    let node = &canvas.nodes[1];
    assert_eq!(node.animate.as_ref().unwrap().ms, 220);
    assert_eq!(node.looping.as_ref().unwrap().period_ms, 800);
    assert!(matches!(&node.kind, Kind::Rect { rect, .. } if matches!(
        &rect[2], Num::Bind { bind, map: Some([0.0,15.0]) } if bind == "work")));
}

#[test]
fn sdk_spike_canvas_bounds_live_declarations() {
    let looping = json!({"rect":[0,0,1,1],"loop":{"kind":"bob","period_ms":800}});
    assert!(with_nodes(json!(vec![looping.clone(); 8])).is_ok());
    assert!(with_nodes(json!(vec![looping; 9])).is_err());
    assert!(with_nodes(json!([{"rect":[0,0,1,1],"loop":{"kind":"bob","period_ms":799}}])).is_err());
    let hit = json!({"rect":[0,0,1,1],"hit":{"copy":"文字"}});
    assert!(with_nodes(json!(vec![hit.clone(); 64])).is_ok());
    assert!(with_nodes(json!(vec![hit; 65])).is_err());
    assert!(with_nodes(json!([{"rect":[0,0,1,1],"hit":{"copy":"岛".repeat(2001)}}])).is_err());
}
