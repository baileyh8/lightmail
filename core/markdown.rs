//! Bounded email tables. html2md's default table handler recursively includes the
//! rows of nested layout tables and pads every row to their expanded width.
//! Never invoke that handler, even when converting an individual cell.
use crate::models::{fail, Result};
use html2md::{Handle, NodeData, StructuredPrinter, TagHandler, TagHandlerFactory};
use std::{cell::Cell, collections::HashMap, rc::Rc};

const MAX_OUTPUT: usize = 8 * 1024 * 1024;
type Handlers = HashMap<String, Box<dyn TagHandlerFactory>>;

pub fn from_html(html: &str) -> Result<String> {
    if html.len() > 4 * 1024 * 1024 {
        return Err(fail("正文超过 4 MiB，无法转换全文"));
    }
    let rejected = Rc::new(Cell::new(false));
    let markdown = html2md::parse_html_custom(html, &handlers(rejected.clone()));
    if rejected.get() || markdown.len() > MAX_OUTPUT {
        return Err(fail(
            "邮件排版结构过于复杂，已停止解析以保护内存；请在邮箱网页查看",
        ));
    }
    Ok(markdown)
}

fn handlers(rejected: Rc<Cell<bool>>) -> Handlers {
    let mut map: Handlers = HashMap::new();
    let error = rejected.clone();
    map.insert(
        "body".into(),
        Box::new(move || BodyGuard {
            rejected: error.clone(),
        }),
    );
    map.insert(
        "table".into(),
        Box::new(move || MailTable {
            skip: false,
            rejected: rejected.clone(),
        }),
    );
    for tag in ["tr", "td", "th"] {
        map.insert(tag.into(), Box::new(|| BlockBoundary));
    }
    for tag in ["head", "style", "script", "title", "img"] {
        map.insert(tag.into(), Box::new(|| SkipContents));
    }
    map.insert("a".into(), Box::new(MailLink::default));
    map
}

struct SkipContents;
impl TagHandler for SkipContents {
    fn handle(&mut self, _: &Handle, _: &mut StructuredPrinter) {}
    fn skip_descendants(&self) -> bool {
        true
    }
    fn after_handle(&mut self, _: &mut StructuredPrinter) {}
}
#[derive(Default)]
struct MailLink {
    start: usize,
    href: String,
}
impl TagHandler for MailLink {
    fn handle(&mut self, node: &Handle, out: &mut StructuredPrinter) {
        self.start = out.data.len();
        if let NodeData::Element { attrs, .. } = &node.data {
            self.href = attrs
                .borrow()
                .iter()
                .find(|a| a.name.local.as_ref() == "href")
                .map(|a| a.value.to_string())
                .unwrap_or_default();
        }
    }
    fn after_handle(&mut self, out: &mut StructuredPrinter) {
        if self.href.is_empty() {
            return;
        }
        if out.data[self.start..].trim().is_empty() {
            out.append_str("打开链接");
        }
        // Never percent-decode a destination: that can corrupt signed URLs.
        let href = self
            .href
            .replace(' ', "%20")
            .replace('(', "%28")
            .replace(')', "%29")
            .replace('<', "%3C")
            .replace('>', "%3E")
            .replace('\\', "%5C");
        out.insert_str(self.start, "[");
        out.append_str(&format!("]({href})"));
    }
}

fn named(node: &Handle, expected: &str) -> bool {
    matches!(&node.data, NodeData::Element { name, .. } if name.local.as_ref() == expected)
}

struct BodyGuard {
    rejected: Rc<Cell<bool>>,
}
impl TagHandler for BodyGuard {
    fn handle(&mut self, root: &Handle, _: &mut StructuredPrinter) {
        let mut stack = vec![(root.clone(), 0)];
        let mut nodes = 0;
        while let Some((node, depth)) = stack.pop() {
            nodes += 1;
            if nodes > 50_000 || depth > 64 {
                self.rejected.set(true);
                return;
            }
            stack.extend(
                node.children
                    .borrow()
                    .iter()
                    .cloned()
                    .map(|child| (child, depth + 1)),
            );
        }
    }
    fn skip_descendants(&self) -> bool {
        self.rejected.get()
    }
    fn after_handle(&mut self, _: &mut StructuredPrinter) {}
}

struct BlockBoundary;
impl TagHandler for BlockBoundary {
    fn handle(&mut self, _: &Handle, out: &mut StructuredPrinter) {
        out.append_str("\n\n");
    }
    fn after_handle(&mut self, out: &mut StructuredPrinter) {
        out.append_str("\n\n");
    }
}

struct MailTable {
    skip: bool,
    rejected: Rc<Cell<bool>>,
}
impl TagHandler for MailTable {
    fn handle(&mut self, table: &Handle, out: &mut StructuredPrinter) {
        out.append_str("\n\n");
        if matches!(&table.data, NodeData::Element { attrs, .. } if attrs.borrow().iter().any(|a|a.name.local.as_ref()=="role" && a.value.as_ref()=="presentation"))
        {
            return;
        }
        // Nested tables are email layout containers, not one giant data table.
        // Let the normal walker visit each descendant once in document order.
        let mut stack = table.children.borrow().clone();
        while let Some(node) = stack.pop() {
            if named(&node, "table") {
                return;
            }
            stack.extend(node.children.borrow().iter().cloned());
        }
        let mut rows = Vec::new();
        for child in table.children.borrow().iter() {
            if named(child, "tr") {
                rows.push(child.clone());
            } else if ["thead", "tbody", "tfoot"]
                .iter()
                .any(|tag| named(child, tag))
            {
                rows.extend(
                    child
                        .children
                        .borrow()
                        .iter()
                        .filter(|node| named(node, "tr"))
                        .cloned(),
                );
            }
        }
        let cells: Vec<Vec<Handle>> = rows
            .iter()
            .map(|row| {
                row.children
                    .borrow()
                    .iter()
                    .filter(|node| named(node, "td") || named(node, "th"))
                    .cloned()
                    .collect()
            })
            .collect();
        let width = cells.iter().map(Vec::len).max().unwrap_or(0);
        // One-column and very wide tables are readable as blocks, with no padding.
        if !(2..=64).contains(&width) || rows.len() < 2 || rows.len() > 2000 {
            return;
        }
        self.skip = true;
        let custom = handlers(self.rejected.clone());
        for (index, row) in cells.iter().enumerate() {
            out.append_str("|");
            for column in 0..width {
                let mut cell = StructuredPrinter::default();
                if let Some(node) = row.get(column) {
                    for child in node.children.borrow().iter() {
                        html2md::walk(child, &mut cell, &custom);
                    }
                }
                let value = cell.data.trim().replace('|', "\\|").replace('\n', "<br>");
                if out.data.len().saturating_add(value.len()) > MAX_OUTPUT {
                    self.rejected.set(true);
                    return;
                }
                out.append_str(&value);
                out.append_str("|");
            }
            out.insert_newline();
            if index == 0 {
                out.append_str("|");
                for _ in 0..width {
                    out.append_str("---|");
                }
                out.insert_newline();
            }
        }
    }
    fn skip_descendants(&self) -> bool {
        self.skip || self.rejected.get()
    }
    fn after_handle(&mut self, out: &mut StructuredPrinter) {
        out.append_str("\n\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_simple_table_and_formatting() {
        let md = from_html("<table><tr><th>Item</th><th>Price</th></tr><tr><td><b>Book</b></td><td><a href='https://example.com'>USD 12.50</a></td></tr></table>").unwrap();
        assert!(md.contains("|Item|Price|\n|---|---|"));
        assert!(md.contains("**Book**"));
        assert!(md.contains("[USD 12.50](https://example.com)"));
    }
    #[test]
    fn sparse_table_does_not_pad_rows_to_the_longest_cell() {
        let html = format!(
            "<table><tr><td>{}</td><td>B</td></tr>{}</table>",
            "x".repeat(100_000),
            "<tr><td>x</td><td>y</td></tr>".repeat(100)
        );
        let md = from_html(&html).unwrap();
        assert!(md.len() < 110_000);
    }
    #[test]
    fn rejects_deep_structure_before_recursive_conversion() {
        let html = format!("{}text{}", "<div>".repeat(100), "</div>".repeat(100));
        assert!(from_html(&html).is_err());
    }
    #[test]
    fn nested_layout_preserves_text_once_links_quotes_and_lists() {
        let content = "<p>UniqueBodyToken</p><p><a href='https://example.com'>Link</a></p><blockquote>Quoted</blockquote><ul><li>One</li><li>Two</li></ul>";
        let html = format!(
            "{}{}{}",
            "<table><tr><td>".repeat(12),
            content,
            "</td></tr></table>".repeat(12)
        );
        let md = from_html(&html).unwrap();
        assert_eq!(md.matches("UniqueBodyToken").count(), 1);
        assert!(md.contains("[Link](https://example.com)"));
        assert!(md.contains("> Quoted"));
        assert!(md.contains("* One") && md.contains("* Two"));
        assert!(md.len() < 1000);
    }
}
