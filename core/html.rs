use html5ever::{local_name, namespace_url, ns, tendril::TendrilSink, Attribute, QualName};
use markup5ever_rcdom::{Handle, Node, NodeData, RcDom, SerializableHandle};
use std::{cell::RefCell, rc::Rc};

pub const RENDER_MARKER: &str = "<!--lightmail-render-v3-->";

// Image-only links must remain usable when remote images are blocked. Parse
// actual HTML nodes so quoted attributes and encoded URLs stay intact.
pub fn restore_link_labels(html: &str) -> String {
    let dom = html5ever::parse_document(RcDom::default(), Default::default()).one(html);
    let mut stack = vec![dom.document.clone()];
    while let Some(node) = stack.pop() {
        stack.extend(node.children.borrow().iter().cloned());
        if !named(&node, "a") || attr(&node, "href").is_empty() {
            continue;
        }
        let mut descendants = node.children.borrow().clone();
        let mut text = String::new();
        let mut image_label = String::new();
        let mut has_image = false;
        while let Some(child) = descendants.pop() {
            if let NodeData::Text { contents } = &child.data {
                text.push_str(&contents.borrow());
            }
            if named(&child, "img") {
                has_image = true;
                if image_label.is_empty() {
                    image_label = attr(&child, "alt");
                }
            }
            descendants.extend(child.children.borrow().iter().cloned());
        }
        if !text.trim().is_empty() {
            continue;
        }
        let label = if image_label.trim().is_empty() {
            "打开链接"
        } else {
            image_label.trim()
        };
        let span = Node::new(NodeData::Element {
            name: QualName::new(None, ns!(html), local_name!("span")),
            attrs: RefCell::new(vec![Attribute {
                name: QualName::new(None, ns!(), local_name!("class")),
                value: if has_image {
                    "lightmail-image-link-label"
                } else {
                    "lightmail-empty-link-label"
                }
                .into(),
            }]),
            template_contents: RefCell::new(None),
            mathml_annotation_xml_integration_point: false,
        });
        let text_node = Node::new(NodeData::Text {
            contents: RefCell::new(label.into()),
        });
        text_node.parent.set(Some(Rc::downgrade(&span)));
        span.children.borrow_mut().push(text_node);
        span.parent.set(Some(Rc::downgrade(&node)));
        node.children.borrow_mut().push(span);
    }
    let mut serialized = Vec::new();
    let root: SerializableHandle = dom.document.into();
    if html5ever::serialize(&mut serialized, &root, Default::default()).is_err() {
        return html.into();
    }
    String::from_utf8(serialized).unwrap_or_else(|_| html.into())
}
fn named(node: &Handle, tag: &str) -> bool {
    matches!(&node.data, NodeData::Element { name, .. } if name.local.as_ref()==tag)
}
fn attr(node: &Handle, key: &str) -> String {
    if let NodeData::Element { attrs, .. } = &node.data {
        return attrs
            .borrow()
            .iter()
            .find(|a| a.name.local.as_ref() == key)
            .map(|a| a.value.to_string())
            .unwrap_or_default();
    }
    String::new()
}
