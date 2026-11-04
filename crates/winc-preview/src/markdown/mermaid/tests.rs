//! mermaid 解析的测试：流程图的形状与连线、状态图、时序图的事件，以及饼图和不受支持的图。

use super::*;

fn flow(source: &str) -> Flow {
    match parse(source) {
        Some(Diagram::Flow(flow)) => flow,
        other => panic!("{other:?}"),
    }
}

#[test]
fn flowchart_nodes_shapes_and_edges() {
    let f = flow(
        "flowchart LR\n  A[开始] --> B{判断?}\n  B -->|是| C([结束])\n  B -- 否 --> D[(库)] & E((圆))\n  C -.-> A\n  D ==> E --- F>旗]\n  %% 注释\n  subgraph 组 [一组]\n    G[/斜/] --x H[\\梯/]\n  end\n",
    );
    assert_eq!(f.dir, Dir::Right);
    let shape = |id: &str| f.nodes.iter().find(|n| n.id == id).unwrap().shape;
    let label = |id: &str| f.nodes.iter().find(|n| n.id == id).unwrap().label.clone();
    assert_eq!(label("A"), "开始");
    assert_eq!(shape("B"), Shape::Diamond);
    assert_eq!(shape("C"), Shape::Stadium);
    assert_eq!(shape("D"), Shape::Cylinder);
    assert_eq!(shape("E"), Shape::Circle);
    assert_eq!(shape("F"), Shape::Asym);
    assert_eq!(shape("G"), Shape::Lean);
    assert_eq!(shape("H"), Shape::TrapezoidDown);
    let edge = |a: &str, b: &str| {
        f.edges
            .iter()
            .find(|e| f.nodes[e.from].id == a && f.nodes[e.to].id == b)
            .unwrap_or_else(|| panic!("{a}->{b}: {:?}", f.edges))
            .clone()
    };
    assert_eq!(edge("B", "C").label, "是");
    assert_eq!(edge("B", "D").label, "否");
    assert_eq!(edge("B", "E").label, "否");
    assert_eq!(edge("C", "A").line, Line::Dotted);
    assert_eq!(edge("D", "E").line, Line::Thick);
    assert_eq!(edge("E", "F").head, Head::None);
    assert_eq!(edge("G", "H").head, Head::Cross);
    assert_eq!(f.groups.len(), 1);
    assert_eq!(f.groups[0].title, "一组");
    assert_eq!(f.groups[0].members.len(), 2);
}

#[test]
fn state_diagrams_become_flows() {
    let f = flow(
        "stateDiagram-v2\n  [*] --> 空闲\n  空闲 --> 运行 : 开始\n  运行 --> [*]\n  state 判断 <<choice>>\n",
    );
    assert_eq!(f.nodes[0].shape, Shape::Start);
    assert_eq!(f.nodes[1].shape, Shape::Round);
    assert_eq!(f.edges[1].label, "开始");
    assert_eq!(f.nodes[3].shape, Shape::End);
    assert_eq!(f.nodes[4].shape, Shape::Diamond);
}

#[test]
fn notes_keep_their_actors_names() {
    // 回归：`note` 后面多一个空格且参与者名是中文时，按小写副本的长度切分原文会切在字符中间，
    // 导致 panic。
    let Some(Diagram::Sequence(s)) = parse(
        "sequenceDiagram\n  note  over 中文: x\n  NOTE Left Of Bob: y\n  Note over 中文,Bob: z\n",
    ) else {
        panic!();
    };
    let names: Vec<&str> = s.actors.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(names, ["中文", "Bob"]);
    let notes: Vec<_> = s
        .events
        .iter()
        .map(|e| match e {
            Event::Note { side, actors, text } => (*side, *actors, text.as_str()),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        notes,
        [
            (Side::Over, (0, 0), "x"),
            (Side::Left, (1, 1), "y"),
            (Side::Over, (0, 1), "z"),
        ]
    );
}

#[test]
fn sequence_messages_notes_and_blocks() {
    let Some(Diagram::Sequence(s)) = parse(
        "sequenceDiagram\n  autonumber\n  participant A as 张三\n  actor B\n  A->>+B: 你好\n  B-->>-A: 好\n  Note right of B: 想一想\n  loop 每分钟\n    A-)B: 心跳\n  end\n  A-xC: 断了\n",
    ) else {
        panic!();
    };
    assert!(s.numbered);
    assert_eq!(s.actors.len(), 3);
    assert_eq!(s.actors[0].label, "张三");
    assert!(s.actors[1].person);
    assert!(matches!(
        &s.events[0],
        Event::Message { from: 0, to: 1, text, dotted: false, arrow: Arrow::Filled, activate: Some(true), .. } if text == "你好"
    ));
    assert!(matches!(
        &s.events[1],
        Event::Message {
            dotted: true,
            activate: Some(false),
            ..
        }
    ));
    assert!(matches!(
        &s.events[2],
        Event::Note {
            side: Side::Right,
            actors: (1, 1),
            ..
        }
    ));
    assert!(
        matches!(&s.events[3], Event::Begin(kind, label) if kind == "loop" && label == "每分钟")
    );
    assert!(matches!(
        &s.events[4],
        Event::Message {
            arrow: Arrow::Open,
            ..
        }
    ));
    assert_eq!(s.events[5], Event::End);
    assert!(matches!(
        &s.events[6],
        Event::Message {
            arrow: Arrow::Cross,
            to: 2,
            ..
        }
    ));
}

#[test]
fn pies_and_unsupported() {
    let Some(Diagram::Pie(p)) = parse("pie showData title 宠物\n  \"狗\" : 386\n  \"猫\" : 85.5\n")
    else {
        panic!();
    };
    assert_eq!(p.title.as_deref(), Some("宠物"));
    assert!(p.show_data);
    assert_eq!(
        p.slices,
        [("狗".to_owned(), 386.0), ("猫".to_owned(), 85.5)]
    );
    assert_eq!(
        parse("classDiagram\n  A <|-- B\n"),
        Some(Diagram::Unsupported("classDiagram".into()))
    );
    assert_eq!(parse("不是图"), None);
}

#[test]
fn too_many_edges_are_left_as_source() {
    // `A & B --> C & D` 生成两组节点数之积条连线：20 × 20 = 400 条低于上限，正常绘制；
    // 30 × 30 = 900 条超过上限，不绘制，按代码块显示原文。
    let chain = |n: usize| {
        let group = |side: &str| {
            (0..n)
                .map(|i| format!("{side}{i}"))
                .collect::<Vec<_>>()
                .join(" & ")
        };
        format!("graph LR\n{} --> {}", group("a"), group("b"))
    };
    let Some(Diagram::Flow(flow)) = parse(&chain(20)) else {
        panic!("400 条连线应当照常画");
    };
    assert_eq!((flow.nodes.len(), flow.edges.len()), (40, 400));
    assert_eq!(parse(&chain(30)), None);
}
