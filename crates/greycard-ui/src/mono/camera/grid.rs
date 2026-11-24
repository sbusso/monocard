//! The camera window's grid, with no window: which tiles show and in what
//! order, the rows they stand in, the selection, and the pictures to ask
//! for.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::ops::Range;

use chrono::NaiveDate;

use crate::mono::usb::event::Tile;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum Sort {
    #[default]
    Newest,
    Oldest,
    Name,
}

impl Sort {
    const ALL: [Sort; 3] = [Sort::Newest, Sort::Oldest, Sort::Name];

    /// Its row of the window's list, and the settings' word.
    pub(super) fn word(self) -> &'static str {
        match self {
            Sort::Newest => "newest",
            Sort::Oldest => "oldest",
            Sort::Name => "name",
        }
    }

    /// Of the settings' word; newest first for any other.
    pub(super) fn named(word: &str) -> Sort {
        let of = |s: &Sort| s.word() == word;
        Sort::ALL.into_iter().find(of).unwrap_or_default()
    }

    pub(super) fn row(self) -> usize {
        Sort::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    pub(super) fn at(row: usize) -> Sort {
        Sort::ALL.get(row).copied().unwrap_or_default()
    }
}

/// The tiles that show, by their place in `tiles`, in order. With `new`,
/// not the imported ones. A tile with no date is after the others.
pub(super) fn shown(tiles: &[Tile], sort: Sort, new: bool) -> Vec<usize> {
    let mut order: Vec<usize> = (0..tiles.len())
        .filter(|i| !new || tiles[*i].imported != Some(true))
        .collect();
    let by_name = |a: &Tile, b: &Tile| {
        let key = |t: &Tile| (t.name.to_lowercase(), t.folder.clone(), t.key);
        key(a).cmp(&key(b))
    };
    order.sort_by(|a, b| {
        let (a, b) = (&tiles[*a], &tiles[*b]);
        let first = match sort {
            Sort::Newest => b.taken.cmp(&a.taken),
            Sort::Oldest => match (a.taken, b.taken) {
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (a, b) => a.cmp(&b),
            },
            Sort::Name => Ordering::Equal,
        };
        first.then_with(|| by_name(a, b))
    });
    order
}

/// The day each tile of `order` is under; none in order of name, where
/// there are no days.
pub(super) fn days(tiles: &[Tile], order: &[usize], sort: Sort) -> Option<Vec<Option<NaiveDate>>> {
    let day = |i: &usize| tiles[*i].taken.map(|t| t.date());
    (sort != Sort::Name).then(|| order.iter().map(day).collect())
}

/// The words over a day.
pub(super) fn day_words(day: Option<NaiveDate>) -> String {
    day.map_or("No date".into(), |d| d.format("%-d %B %Y").to_string())
}

/// The tiles' size: the least width of one.
pub(super) const LEAST: f32 = 120.0;
pub(super) const MOST: f32 = 320.0;
pub(super) const USUAL: f32 = 160.0;

/// The room round the tiles and between them, under a picture for the two
/// lines, and of a day's words.
const PAD: f32 = 16.0;
const GAP: f32 = 8.0;
const WORDS: f32 = 44.0;
const HEAD: f32 = 36.0;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Day,
    Tiles,
}

/// A day's words, over its first tile, or a row of tiles.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    kind: Kind,
    top: f32,
    /// The tiles of the row, or of the day.
    tiles: Range<usize>,
}

/// Where everything is.
#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct Layout {
    rows: Vec<Row>,
    columns: usize,
    pub(super) width: f32,
    pub(super) tile: (f32, f32),
    pub(super) height: f32,
}

/// Which way the keys go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    Left,
    Right,
    Up,
    Down,
    First,
    Last,
}

impl Layout {
    /// `count` tiles at least `size` wide in `width`, under their days.
    pub(super) fn new(
        count: usize,
        days: Option<&[Option<NaiveDate>]>,
        width: f32,
        size: f32,
    ) -> Layout {
        let room = (width - 2.0 * PAD).max(size);
        let columns = (((room + GAP) / (size + GAP)) as usize).max(1);
        let wide = ((room + GAP) / columns as f32 - GAP).floor();
        // A picture of 3 to 2 fills the box, and an upright one its height.
        let tall = (wide * 0.75).round() + WORDS;
        let mut rows = Vec::new();
        let mut top = PAD;
        let mut at = 0;
        while at < count {
            let end = match days {
                Some(d) => at + d[at..].iter().take_while(|x| **x == d[at]).count(),
                None => count,
            };
            if days.is_some() {
                rows.push(Row {
                    kind: Kind::Day,
                    top,
                    tiles: at..end,
                });
                top += HEAD;
            }
            while at < end {
                let next = (at + columns).min(end);
                rows.push(Row {
                    kind: Kind::Tiles,
                    top,
                    tiles: at..next,
                });
                top += tall + GAP;
                at = next;
            }
        }
        Layout {
            rows,
            columns,
            width: room,
            tile: (wide, tall),
            height: top + PAD - GAP,
        }
    }

    fn row_height(&self, row: &Row) -> f32 {
        match row.kind {
            Kind::Day => HEAD,
            Kind::Tiles => self.tile.1 + GAP,
        }
    }

    /// The rows with anything from `top` to `bottom`.
    fn rows_in(&self, top: f32, bottom: f32) -> &[Row] {
        let from = self
            .rows
            .partition_point(|r| r.top + self.row_height(r) <= top);
        let to = self.rows.partition_point(|r| r.top < bottom);
        &self.rows[from..to.max(from)]
    }

    /// The tiles from `top` to `bottom`.
    pub(super) fn tiles_in(&self, top: f32, bottom: f32) -> Range<usize> {
        let mut rows = self
            .rows_in(top, bottom)
            .iter()
            .filter(|r| r.kind == Kind::Tiles);
        let Some(first) = rows.next() else {
            return 0..0;
        };
        first.tiles.start..rows.next_back().unwrap_or(first).tiles.end
    }

    /// The days' words from `top` to `bottom`: where each is, and its tiles.
    pub(super) fn heads_in(
        &self,
        top: f32,
        bottom: f32,
    ) -> impl Iterator<Item = (f32, Range<usize>)> {
        self.rows_in(top, bottom)
            .iter()
            .filter(|r| r.kind == Kind::Day)
            .map(|r| (r.top, r.tiles.clone()))
    }

    /// The row of tiles that tile `at` is in, by its place in `rows`.
    fn row_of(&self, at: usize) -> Option<usize> {
        // A day and its first row begin with one tile, the day before.
        let row = self.rows.partition_point(|r| r.tiles.start <= at);
        row.checked_sub(1)
    }

    /// The top left of tile `at`.
    pub(super) fn place(&self, at: usize) -> (f32, f32) {
        let Some(row) = self.row_of(at).map(|i| &self.rows[i]) else {
            return (PAD, PAD);
        };
        let column = (at - row.tiles.start) as f32;
        (PAD + column * (self.tile.0 + GAP), row.top)
    }

    /// The tile a step from `at` goes to.
    pub(super) fn step(&self, at: usize, step: Step) -> usize {
        let last = self.rows.last().map_or(0, |r| r.tiles.end - 1);
        let Some(row) = self.row_of(at) else {
            return at;
        };
        let column = at - self.rows[row].tiles.start;
        let over =
            |to: Option<&Row>| to.map_or(at, |r| (r.tiles.start + column).min(r.tiles.end - 1));
        let tiles = |r: &&Row| r.kind == Kind::Tiles;
        match step {
            Step::Left => at.saturating_sub(1),
            Step::Right => (at + 1).min(last),
            Step::Up => over(self.rows[..row].iter().rfind(tiles)),
            Step::Down => over(self.rows[row + 1..].iter().find(tiles)),
            Step::First => 0,
            Step::Last => last,
        }
    }

    /// The scroll, from `top`, at which tile `at` is in a view `height`
    /// tall, its day's words with it in the first row.
    pub(super) fn reveal(&self, at: usize, top: f32, height: f32) -> f32 {
        let (_, y) = self.place(at);
        let row = self.row_of(at).unwrap_or(0);
        let first = row > 0 && self.rows[row - 1].kind == Kind::Day;
        let from = if first || row == 0 {
            y - HEAD - PAD
        } else {
            y - GAP
        };
        let to = (y + self.tile.1 + PAD - height).min(from);
        self.held(top.clamp(to, from), height)
    }

    /// A scroll held to what there is.
    pub(super) fn held(&self, top: f32, height: f32) -> f32 {
        top.min(self.height - height).max(0.0)
    }
}

/// The tiles that are chosen, the one the keys are on, and the one a range
/// is from; by their keys.
#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct Selection {
    keys: HashSet<u64>,
    pub(super) on: Option<u64>,
    from: Option<u64>,
}

impl Selection {
    pub(super) fn has(&self, key: u64) -> bool {
        self.keys.contains(&key)
    }

    pub(super) fn len(&self) -> usize {
        self.keys.len()
    }

    /// Nothing chosen; the keys stay where they are.
    pub(super) fn clear(&mut self) {
        self.keys.clear();
    }

    pub(super) fn all(&mut self, shown: &[u64]) {
        self.keys = shown.iter().copied().collect();
    }

    /// Only what shows is chosen.
    pub(super) fn keep(&mut self, shown: &[u64]) {
        let there: HashSet<u64> = shown.iter().copied().collect();
        self.keys.retain(|k| there.contains(k));
        self.on = self.on.filter(|k| there.contains(k));
        self.from = self.from.filter(|k| there.contains(k));
    }

    fn one(&mut self, key: u64) {
        self.keys = [key].into();
        (self.on, self.from) = (Some(key), Some(key));
    }

    /// From where a range began to `key`, with what was chosen when `more`.
    fn range(&mut self, shown: &[u64], key: u64, more: bool) {
        let place = |k: Option<u64>| shown.iter().position(|s| Some(*s) == k);
        let Some(to) = place(Some(key)) else { return };
        let from = place(self.from.or(self.on)).unwrap_or(to);
        if !more {
            self.keys.clear();
        }
        self.keys.extend(&shown[from.min(to)..=from.max(to)]);
        (self.on, self.from) = (Some(key), Some(shown[from]));
    }

    /// A click on `key`.
    pub(super) fn click(&mut self, shown: &[u64], key: u64, command: bool, shift: bool) {
        match (command, shift) {
            (more, true) => self.range(shown, key, more),
            (true, false) => {
                (self.on, self.from) = (Some(key), Some(key));
                self.toggle();
            }
            (false, false) => self.one(key),
        }
    }

    /// The tile the keys are on, chosen or not any more.
    pub(super) fn toggle(&mut self) {
        if let Some(key) = self.on
            && !self.keys.remove(&key)
        {
            self.keys.insert(key);
        }
    }

    /// The keys go to `key`.
    pub(super) fn step(&mut self, shown: &[u64], key: u64, shift: bool) {
        if shift {
            self.range(shown, key, false);
        } else {
            self.one(key);
        }
    }
}

/// The pictures to ask for: those on screen, then those after, then those
/// before, without the ones that came.
pub(super) fn wanted(
    shown: &[u64],
    screen: &Range<usize>,
    around: &Range<usize>,
    came: impl Fn(u64) -> bool,
) -> Vec<u64> {
    let after = screen.end.max(around.start)..around.end;
    let before = around.start..screen.start.min(around.end);
    let ask = screen.clone().chain(after).chain(before.rev());
    ask.filter_map(|i| shown.get(i).copied())
        .filter(|k| !came(*k))
        .collect()
}

/// What Import all takes: what shows and is not known to be imported.
pub(super) fn not_imported<'a>(
    tiles: &'a [Tile],
    order: &'a [usize],
) -> impl Iterator<Item = u64> + 'a {
    let new = |i: &&usize| tiles[**i].imported != Some(true);
    order.iter().filter(new).map(|i| tiles[*i].key)
}
