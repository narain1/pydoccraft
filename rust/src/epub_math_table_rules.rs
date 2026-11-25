//! Measured MathML rules and editable Word cell borders share one span map.
use roxmltree::Node;
use wordcraft_doc::{
    props::{Border, BorderStyle, Borders, Rgb},
    Table,
};
use wordcraft_layout::math::MItem;

use super::{indexed, length, limited, unsupported, Cell};
use crate::EngineError;

pub(super) const RULE_WIDTH: f32 = 0.75;
const HALF_RULE: f32 = RULE_WIDTH / 2.0;
pub(super) const MAX_RULE_ITEMS: usize = 24_000;

pub(super) struct Rules {
    rows: Vec<BorderStyle>,
    columns: Vec<BorderStyle>,
    frame: BorderStyle,
    padding: [f32; 2],
}

pub(super) fn requested(table: Node<'_, '_>) -> bool {
    ["frame", "rowlines", "columnlines"]
        .into_iter()
        .any(|name| {
            table
                .attribute(name)
                .is_some_and(|value| value.split_ascii_whitespace().any(|style| style != "none"))
        })
}

impl Rules {
    pub(super) fn read(
        table: Node<'_, '_>,
        em: f32,
        row_gaps: &mut [f32],
        column_gaps: &mut [f32],
    ) -> Result<Self, EngineError> {
        let frame = parse_style(table.attribute("frame").unwrap_or("none").trim())?;
        let mut padding = [0.0; 2];
        if frame != BorderStyle::None {
            let mut values = table
                .attribute("framespacing")
                .unwrap_or("0.4em 0.5ex")
                .split_ascii_whitespace();
            for axis in &mut padding {
                *axis = values
                    .next()
                    .and_then(|value| length(value, em))
                    .ok_or_else(|| unsupported("MathML table frame spacing"))?;
            }
            if values.next().is_some() {
                return Err(unsupported("MathML table frame spacing"));
            }
        }
        let decorate = |name: &str, gaps: &mut [f32]| {
            gaps.iter_mut()
                .enumerate()
                .map(|(index, gap)| {
                    let style =
                        parse_style(indexed(table.attribute(name), index).unwrap_or("none"))?;
                    *gap += thickness(style);
                    Ok(style)
                })
                .collect::<Result<_, EngineError>>()
        };
        Ok(Self {
            rows: decorate("rowlines", row_gaps)?,
            columns: decorate("columnlines", column_gaps)?,
            frame,
            padding,
        })
    }

    pub(super) fn visible(&self) -> bool {
        self.frame != BorderStyle::None
            || self
                .rows
                .iter()
                .chain(&self.columns)
                .any(|&style| style != BorderStyle::None)
    }

    pub(super) fn blocks_label_band(&self, cut: usize) -> bool {
        self.frame != BorderStyle::None
            || self.rows[cut] != BorderStyle::None
            || self.columns.iter().any(|&style| style != BorderStyle::None)
    }

    pub(super) fn inset(&self) -> [f32; 2] {
        self.padding.map(|size| size + thickness(self.frame))
    }

    pub(super) fn native_margin(&self) -> [f32; 2] {
        self.padding.map(|size| size + thickness(self.frame) / 2.0)
    }

    pub(super) fn native_vertical_bands(&self, start: usize, end: usize) -> [f32; 2] {
        let top = start
            .checked_sub(1)
            .map_or(self.frame, |index| self.rows[index]);
        let bottom = if end == self.rows.len() + 1 {
            self.frame
        } else {
            BorderStyle::None
        };
        [thickness(top), thickness(bottom)]
    }

    pub(super) fn draw(
        &self,
        starts: [&[f32]; 2],
        gaps: [&[f32]; 2],
        occupancy: &Occupancy,
        baseline: f32,
        color: Rgb,
        remaining: &mut usize,
    ) -> Result<Vec<MItem>, EngineError> {
        let frame = self.frame != BorderStyle::None;
        let inset = self.inset();
        let edges = |axis: usize| {
            let mut result = Vec::with_capacity(starts[axis].len());
            let edge = if frame { HALF_RULE } else { 0.0 };
            result.push(edge);
            for (&start, &gap) in starts[axis].iter().skip(1).zip(gaps[axis]) {
                result.push(inset[axis] + start - gap / 2.0);
            }
            result
                .push(inset[axis] * 2.0 + starts[axis].last().copied().unwrap_or_default() - edge);
            result
        };
        let columns = edges(0);
        let rows = edges(1);
        let mut paint = Paint {
            items: Vec::new(),
            baseline,
            color,
            remaining,
        };
        if frame {
            for &x in &[columns[0], columns[columns.len() - 1]] {
                paint.line(true, x, rows[0], rows[rows.len() - 1], self.frame)?;
            }
            for &y in &[rows[0], rows[rows.len() - 1]] {
                paint.line(
                    false,
                    y,
                    0.0,
                    columns[columns.len() - 1] + HALF_RULE,
                    self.frame,
                )?;
            }
        }
        for (vertical, styles, parallel, perpendicular) in [
            (true, &self.columns, &columns, &rows),
            (false, &self.rows, &rows, &columns),
        ] {
            for (cut, &style) in styles.iter().enumerate() {
                if style == BorderStyle::None {
                    continue;
                }
                let mut start = 0;
                let segments = perpendicular.len() - 1;
                for end in 0..=segments {
                    if end == segments || occupancy.crosses(vertical, cut, end) {
                        paint.line(
                            vertical,
                            parallel[cut + 1],
                            perpendicular[start],
                            perpendicular[end],
                            style,
                        )?;
                        start = end + 1;
                    }
                }
            }
        }
        Ok(paint.items)
    }

    pub(super) fn set_native_edges(
        &self,
        table: &mut Table,
        map: &Occupancy,
        color: Rgb,
        data_start: usize,
        data_rows: &[usize],
    ) {
        let frame = border(self.frame, color);
        let cell_frame = if data_start == 0 && data_rows.len() == table.rows.len() {
            table.props.borders = Some(frame.map_or_else(Borders::default, Borders::box_));
            None
        } else {
            frame
        };
        let [nrows, ncols] = [data_rows.len(), map.columns];
        for (r, &physical) in data_rows.iter().enumerate() {
            let row = &mut table.rows[physical];
            let mut col = 0usize;
            for cell in &mut row.cells {
                let start = col.checked_sub(data_start);
                col += cell.span();
                let Some(start) = start.filter(|&start| start < ncols) else {
                    continue;
                };
                let end = col - data_start;
                let edge = |vertical: bool, boundary: usize, track: usize| {
                    let styles = if vertical { &self.columns } else { &self.rows };
                    styles.get(boundary).and_then(|style| {
                        (!map.crosses(vertical, boundary, track))
                            .then(|| border(*style, color))
                            .flatten()
                    })
                };
                let borders = Borders {
                    top: r
                        .checked_sub(1)
                        .map_or(cell_frame, |cut| edge(false, cut, start)),
                    bottom: if r + 1 == nrows {
                        cell_frame
                    } else {
                        edge(false, r, start)
                    },
                    left: start
                        .checked_sub(1)
                        .map_or(cell_frame, |cut| edge(true, cut, r)),
                    right: if end == ncols {
                        cell_frame
                    } else {
                        edge(true, end - 1, r)
                    },
                    ..Default::default()
                };
                if borders.any_visible() {
                    cell.props.borders = Some(borders);
                }
            }
        }
    }
}

pub(super) struct Occupancy {
    columns: usize,
    owners: Vec<Option<usize>>,
}

impl Occupancy {
    pub(super) fn new(cells: &[Cell], rows: usize, columns: usize) -> Self {
        let mut owners = vec![None; rows * columns];
        for (index, cell) in cells.iter().enumerate() {
            for row in cell.rows.clone() {
                owners[row * columns + cell.columns.start..row * columns + cell.columns.end]
                    .fill(Some(index));
            }
        }
        Self { columns, owners }
    }

    fn crosses(&self, vertical: bool, cut: usize, track: usize) -> bool {
        let (row, col, step) = if vertical {
            (track, cut, 1)
        } else {
            (cut, track, self.columns)
        };
        let index = row * self.columns + col;
        self.owners[index].is_some() && self.owners[index] == self.owners[index + step]
    }
}

struct Paint<'a> {
    items: Vec<MItem>,
    baseline: f32,
    color: Rgb,
    remaining: &'a mut usize,
}

impl Paint<'_> {
    fn line(
        &mut self,
        vertical: bool,
        center: f32,
        mut start: f32,
        end: f32,
        style: BorderStyle,
    ) -> Result<(), EngineError> {
        let dash = style == BorderStyle::Dashed;
        while start < end {
            if *self.remaining == 0 {
                return Err(limited("MathML table rules exceed the paint budget"));
            }
            *self.remaining -= 1;
            let stop = if dash {
                (start + 3.0 * RULE_WIDTH).min(end)
            } else {
                end
            };
            let (x, y, w, h) = if vertical {
                (
                    center - HALF_RULE,
                    self.baseline - stop,
                    RULE_WIDTH,
                    stop - start,
                )
            } else {
                (
                    start,
                    self.baseline - center - HALF_RULE,
                    stop - start,
                    RULE_WIDTH,
                )
            };
            self.items.push(MItem::Rect {
                x,
                y,
                w,
                h,
                color: self.color,
            });
            start = stop + RULE_WIDTH;
        }
        Ok(())
    }
}

fn parse_style(value: &str) -> Result<BorderStyle, EngineError> {
    match value {
        "none" => Ok(BorderStyle::None),
        "solid" => Ok(BorderStyle::Single),
        "dashed" => Ok(BorderStyle::Dashed),
        _ => Err(unsupported("MathML table rule style")),
    }
}

fn thickness(style: BorderStyle) -> f32 {
    if style == BorderStyle::None {
        0.0
    } else {
        RULE_WIDTH
    }
}

fn border(style: BorderStyle, color: Rgb) -> Option<Border> {
    (style != BorderStyle::None).then_some(Border {
        style,
        width: RULE_WIDTH,
        color: Some(color),
        space: 0.0,
    })
}
