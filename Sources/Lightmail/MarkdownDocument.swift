import Foundation
import SwiftUI

struct MarkdownPiece: Identifiable {
  let id: Int
  var text = AttributedString()
  var heading: Int?
  var quoteDepth = 0
  var prefix = ""
  var indent = 0
  var code = false
  var tableID: Int?
  var row = 0
  var column = 0
  var font: Font {
    if code { return .system(size: 13, design: .monospaced) }
    let size = CGFloat(heading.map { max(16, 27 - $0 * 2) } ?? 15)
    return .system(size: size, weight: heading == nil ? .regular : .semibold)
  }
}

enum MarkdownBlock: Identifiable {
  case paragraph(MarkdownPiece)
  case table(Int, [[AttributedString]])
  var id: Int {
    switch self { case .paragraph(let p): return p.id; case .table(let id, _): return id }
  }
}

enum MarkdownDocument {
  static func parse(_ source: String) -> [MarkdownBlock] {
    // GFM uses inline <br> inside table cells. Apple's Markdown parser
    // exposes it as text; retain the cell break without creating a new row.
    var fenced = false
    let normalized = source.components(separatedBy: "\n").map { line -> String in
      let trimmed = line.trimmingCharacters(in: .whitespaces)
      if trimmed.hasPrefix("```") || trimmed.hasPrefix("~~~") { fenced.toggle() }
      guard !fenced, trimmed.hasPrefix("|") else { return line }
      return line.replacingOccurrences(of: "(?i)<br\\s*/?>", with: "\u{2028}", options: .regularExpression)
    }.joined(separator: "\n")
    guard let parsed = try? AttributedString(markdown: normalized,
      options: .init(interpretedSyntax: .full, failurePolicy: .returnPartiallyParsedIfPossible)) else {
      return [.paragraph(MarkdownPiece(id: 0, text: AttributedString(source)))]
    }
    var pieces: [MarkdownPiece] = []
    var listed: Set<Int> = []
    for run in parsed.runs {
      let intents = run.presentationIntent?.components ?? []
      let id = intents.first?.identity ?? -1
      var value = AttributedString(parsed[run.range])
      value.presentationIntent = nil
      if pieces.last?.id == id { pieces[pieces.count - 1].text.append(value); continue }
      var piece = MarkdownPiece(id: id, text: value)
      var item: (Int, Int)?
      var ordered = false
      for intent in intents {
        switch intent.kind {
        case .header(let level): piece.heading = level
        case .blockQuote: piece.quoteDepth += 1
        case .codeBlock: piece.code = true
        case .listItem(let ordinal): if item == nil { item = (intent.identity, ordinal) }
        case .orderedList: piece.indent += 1; if piece.indent == 1 { ordered = true }
        case .unorderedList: piece.indent += 1
        case .table: piece.tableID = intent.identity
        case .tableHeaderRow: piece.row = 0
        case .tableRow(let row): piece.row = row
        case .tableCell(let column): piece.column = column
        default: break
        }
      }
      if let item, listed.insert(item.0).inserted { piece.prefix = ordered ? "\(item.1)." : "•" }
      pieces.append(piece)
    }
    var blocks: [MarkdownBlock] = []
    for piece in pieces {
      if let table = piece.tableID {
        var rows: [[AttributedString]] = []
        if case .table(let old, let previous)? = blocks.last, old == table {
          rows = previous; blocks.removeLast()
        }
        while rows.count <= piece.row { rows.append([]) }
        while rows[piece.row].count <= piece.column { rows[piece.row].append(AttributedString()) }
        rows[piece.row][piece.column].append(piece.text)
        blocks.append(.table(table, rows))
      } else { blocks.append(.paragraph(piece)) }
    }
    return blocks
  }
}

struct MarkdownBody: View {
  let markdown: String
  @State private var blocks: [MarkdownBlock] = []
  var body: some View {
    VStack(alignment: .leading, spacing: 14) {
      ForEach(blocks) { block in
        switch block {
        case .paragraph(let p):
          HStack(alignment: .top, spacing: 10) {
            if p.quoteDepth > 0 { Rectangle().fill(Theme.line).frame(width: 3) }
            if !p.prefix.isEmpty { Text(p.prefix).foregroundStyle(Theme.muted).frame(minWidth: 16, alignment: .trailing) }
            Text(p.text)
              .font(p.font)
              .lineSpacing(6).frame(maxWidth: .infinity, alignment: .leading)
              .padding(p.code ? 12 : 0)
              .background(p.code ? Theme.sidebar : Color.clear, in: RoundedRectangle(cornerRadius: 6))
          }.padding(.leading, CGFloat(max(0, p.indent - 1)) * 18)
            .fixedSize(horizontal: false, vertical: true)
        case .table(_, let rows):
          ScrollView(.horizontal) {
            Grid(alignment: .leading, horizontalSpacing: 0, verticalSpacing: 0) {
              ForEach(Array(rows.enumerated()), id: \.offset) { rowIndex, cells in
                GridRow {
                  ForEach(Array(cells.enumerated()), id: \.offset) { _, cell in
                    Text(cell).font(.system(size: 13, weight: rowIndex == 0 ? .semibold : .regular))
                      .padding(10).frame(maxWidth: .infinity, alignment: .leading)
                      .background(rowIndex == 0 ? Theme.sidebar : Color.clear)
                      .overlay(Rectangle().stroke(Theme.line, lineWidth: 0.5))
                  }
                }
              }
            }
          }.fixedSize(horizontal: false, vertical: true)
        }
      }
    }.foregroundStyle(Theme.ink).textSelection(.enabled)
      .frame(maxWidth: .infinity, alignment: .leading)
      .task(id: markdown) {
        let source = markdown
        let parsed = await Task.detached(priority: .userInitiated) { MarkdownDocument.parse(source) }.value
        guard !Task.isCancelled else { return }
        blocks = parsed
      }
  }
}
