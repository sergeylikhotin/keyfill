//! Sourcemap бандла (v3): переводит строку и столбец бандла в место в исходнике TSX.
//! Нужен для исключений JS: стек QuickJS указывает на бандл, а агенту нужен его файл.

use keyfill_js::JsException;
use serde::Deserialize;

#[derive(Deserialize)]
struct Raw {
    sources: Vec<String>,
    mappings: String,
}

/// Отрезок строки бандла: с этого столбца код пришёл из `source:line:col` (всё с нуля).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Segment {
    col: u32,
    source: u32,
    line: u32,
    src_col: u32,
}

#[derive(Debug, Clone)]
pub struct SourceMap {
    sources: Vec<String>,
    lines: Vec<Vec<Segment>>,
}

/// Место в исходнике, строки и столбцы с единицы.
#[derive(Debug, Clone, PartialEq)]
pub struct Origin {
    pub file: String,
    pub line: u32,
    pub column: u32,
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.column)
    }
}

fn vlq(chars: &mut std::iter::Peekable<std::str::Bytes<'_>>) -> Option<i64> {
    let mut value: i64 = 0;
    let mut shift = 0;
    loop {
        let c = chars.next()?;
        let digit = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as i64;
        value |= (digit & 31) << shift;
        if digit & 32 == 0 {
            break;
        }
        shift += 5;
    }
    Some(if value & 1 == 1 { -(value >> 1) } else { value >> 1 })
}

impl SourceMap {
    pub fn parse(json: &serde_json::Value) -> Option<SourceMap> {
        let raw: Raw = serde_json::from_value(json.clone()).ok()?;
        let mut lines = Vec::new();
        let (mut source, mut line, mut src_col) = (0i64, 0i64, 0i64);
        for text in raw.mappings.split(';') {
            let mut segs = Vec::new();
            let mut col = 0i64;
            for seg in text.split(',').filter(|s| !s.is_empty()) {
                let mut it = seg.bytes().peekable();
                col += vlq(&mut it)?;
                if it.peek().is_none() {
                    continue;
                }
                source += vlq(&mut it)?;
                line += vlq(&mut it)?;
                src_col += vlq(&mut it)?;
                segs.push(Segment { col: col as u32, source: source as u32, line: line as u32, src_col: src_col as u32 });
            }
            lines.push(segs);
        }
        Some(SourceMap { sources: raw.sources, lines })
    }

    /// Место в исходнике для строки и столбца бандла (с единицы, как в стеке QuickJS).
    pub fn lookup(&self, line: u32, column: u32) -> Option<Origin> {
        let segs = self.lines.get(line.checked_sub(1)? as usize)?;
        let col = column.saturating_sub(1);
        let seg = segs.iter().rev().find(|s| s.col <= col).or(segs.first())?;
        Some(Origin { file: self.sources.get(seg.source as usize)?.clone(), line: seg.line + 1, column: seg.src_col + 1 })
    }

    /// Переводит кадры стека бандла (`eval_script:250:12`) в места TSX. Кадры реализации
    /// `@keyfill/core` и обвязки бандла скрываются: агенту нужен свой код. Без места у исключения
    /// оно берётся из первого оставшегося кадра.
    pub fn map_exception(&self, ex: &mut JsException) {
        let frames = std::mem::take(&mut ex.stack);
        for mut frame in frames {
            if frame.file != "eval_script" {
                ex.stack.push(frame);
                continue;
            }
            match self.lookup(frame.line, frame.column) {
                Some(o) if !(o.file.contains("packages/core/") || o.file.ends_with("keyfill-entry.ts")) => {
                    (frame.file, frame.line, frame.column) = (o.file, o.line, o.column);
                    ex.stack.push(frame);
                }
                _ => ex.hidden += 1,
            }
        }
        if ex.source.is_none() {
            ex.source = ex.stack.first().map(|f| format!("{}:{}:{}", f.file, f.line, f.column));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_vlq_and_looks_up() {
        // Строка 1: столбец 0 → a.ts:1:1, столбец 4 → a.ts:2:3; строка 2: столбец 2 → b.ts:1:1.
        let map = serde_json::json!({ "version": 3, "sources": ["a.ts", "b.ts"], "mappings": "AAAA,IACE;ECDF" });
        let m = SourceMap::parse(&map).unwrap();
        assert_eq!(m.lookup(1, 1).unwrap().to_string(), "a.ts:1:1");
        assert_eq!(m.lookup(1, 7).unwrap().to_string(), "a.ts:2:3");
        assert_eq!(m.lookup(2, 3).unwrap().to_string(), "b.ts:1:1");
        assert!(m.lookup(9, 1).is_none());
    }
}
