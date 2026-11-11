//! Shelf packing with least-recently-used eviction, shared by the glyph
//! atlas (`text.rs`) and the image atlas (`image.rs`).
//!
//! Items go into rows ("shelves") whose heights are rounded up to a bucket,
//! so an emptied shelf fits other items of the same size class. When the
//! texture is full, the shelf used least recently is emptied and reused,
//! but never one drawn in the current frame: its texels are already
//! referenced by this frame's instances. An item taller than every shelf
//! that could be emptied takes several neighbouring ones, joined into one:
//! a full atlas of small things still has room for a large one. Only when
//! what is in use leaves no such room does allocation fail, and the caller
//! wipes the atlas and redraws.

/// Shelf heights are multiples of this, in texels.
const BUCKET: u32 = 8;

/// Marks an item that occupies no shelf (a glyph without ink).
pub const NO_SHELF: u16 = u16::MAX;

/// One row of the atlas.
#[derive(Clone, Copy, Debug)]
struct Shelf {
    /// Top edge.
    y: u32,
    /// Height, a multiple of [`BUCKET`].
    h: u32,
    /// Where the next item goes.
    x: u32,
    /// Frame the shelf was last drawn from.
    used: u64,
}

/// A place in the atlas returned by [`Shelves::alloc`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    /// Left edge in texels.
    pub x: u32,
    /// Top edge in texels.
    pub y: u32,
    /// Shelf holding it, for [`Shelves::touch`].
    pub shelf: u16,
    /// The shelves emptied to make room: everything in them must be forgotten.
    pub evicted: Vec<u16>,
}

/// Allocator for a square texture of `size` texels.
#[derive(Debug)]
pub struct Shelves {
    size: u32,
    rows: Vec<Shelf>,
    /// Top of the unused space below the last shelf.
    bottom: u32,
    frame: u64,
}

impl Shelves {
    /// An empty atlas of `size`×`size` texels.
    #[must_use]
    pub fn new(size: u32) -> Self {
        Self { size, rows: Vec::new(), bottom: 0, frame: 1 }
    }

    /// Starts a frame: shelves touched from now on count as in use.
    pub fn next_frame(&mut self) {
        self.frame += 1;
    }

    /// Records that an item on shelf `index` is drawn this frame.
    pub fn touch(&mut self, index: u16) {
        if let Some(s) = self.rows.get_mut(usize::from(index)) {
            s.used = self.frame;
        }
    }

    /// Finds room for a `w`×`h` item (padding included by the caller).
    /// `None` means what is in use this frame leaves no room for it.
    pub fn alloc(&mut self, w: u32, h: u32) -> Option<Slot> {
        if w > self.size || h > self.size || self.rows.len() >= usize::from(NO_SHELF) {
            return None;
        }
        let bucket = h.div_ceil(BUCKET).max(1) * BUCKET;
        let frame = self.frame;
        let fits = |s: &Shelf| s.h == bucket && s.x + w <= self.size;
        // Least recently used shelf that is tall enough, preferring the
        // same size class so tall shelves are not wasted on small items.
        let idle = |rows: &[Shelf]| {
            let tall = rows.iter().enumerate().filter(|(_, s)| s.h >= bucket && s.used < frame);
            tall.min_by_key(|(_, s)| (s.h != bucket, s.used)).map(|(i, _)| i)
        };
        let mut evicted = Vec::new();
        let index = if let Some(index) = self.rows.iter().position(fits) {
            index
        } else if self.bottom + bucket <= self.size {
            self.rows.push(Shelf { y: self.bottom, h: bucket, x: 0, used: 0 });
            self.bottom += bucket;
            self.rows.len() - 1
        } else if let Some(index) = idle(&self.rows) {
            self.rows[index].x = 0;
            evicted.push(index as u16);
            index
        } else {
            self.join(bucket, &mut evicted)?
        };
        let row = &mut self.rows[index];
        let slot = Slot { x: row.x, y: row.y, shelf: index as u16, evicted };
        row.x += w;
        row.used = self.frame;
        Some(slot)
    }

    /// Makes a shelf `bucket` tall out of neighbouring shelves, none of
    /// them in use this frame, that are tall enough only together: the run
    /// whose latest use is the longest ago. They are noted in `evicted`.
    /// The first becomes the new shelf, the second what is left over of
    /// their height, the others are no more (their height is zero, so
    /// nothing is put in them again). Returns the new shelf.
    fn join(&mut self, bucket: u32, evicted: &mut Vec<u16>) -> Option<usize> {
        let frame = self.frame;
        // The shelves from the top down: each starts where the one before it ends.
        let mut order: Vec<usize> = (0..self.rows.len()).filter(|&index| self.rows[index].h > 0).collect();
        order.sort_unstable_by_key(|&index| self.rows[index].y);
        let mut best: Option<(u64, usize, usize)> = None;
        for start in 0..order.len() {
            let (mut height, mut latest) = (0, 0);
            for (end, &index) in order.iter().enumerate().skip(start) {
                let row = self.rows[index];
                if row.used >= frame {
                    break;
                }
                height += row.h;
                latest = latest.max(row.used);
                if height >= bucket {
                    if best.is_none_or(|(used, ..)| latest < used) {
                        best = Some((latest, start, end));
                    }
                    break;
                }
            }
        }
        let (_, start, end) = best?;
        let run = &order[start..=end];
        let top = self.rows[run[0]].y;
        let total: u32 = run.iter().map(|&index| self.rows[index].h).sum();
        evicted.extend(run.iter().map(|&index| index as u16));
        for (place, &index) in run.iter().enumerate() {
            self.rows[index] = match place {
                0 => Shelf { y: top, h: bucket, x: 0, used: 0 },
                1 => Shelf { y: top + bucket, h: total - bucket, x: 0, used: 0 },
                _ => Shelf { y: 0, h: 0, x: 0, used: 0 },
            };
        }
        Some(run[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_rows_and_evicts_the_least_recently_used() {
        let mut atlas = Shelves::new(32);
        // Four 16×16 items fill a 32×32 atlas in two shelves.
        let a = atlas.alloc(16, 16).unwrap();
        let b = atlas.alloc(16, 16).unwrap();
        assert_eq!((a.shelf, b.shelf, b.x), (0, 0, 16));
        atlas.alloc(16, 16).unwrap();
        atlas.alloc(16, 16).unwrap();
        assert!(atlas.alloc(16, 16).is_none(), "everything is in use this frame");

        atlas.next_frame();
        atlas.touch(0);
        let c = atlas.alloc(16, 16).unwrap();
        assert_eq!((c.shelf, c.evicted.as_slice(), c.x, c.y), (1, &[1][..], 0, 16), "the untouched shelf is reused");
        // Items taller than the atlas never fit.
        assert!(atlas.alloc(8, 64).is_none());
    }

    #[test]
    fn a_tall_item_takes_neighbouring_shelves_none_of_which_is_tall_enough() {
        let mut atlas = Shelves::new(64);
        // An atlas full of low shelves: eight of them, each filled by one wide item.
        for at in 0..8 {
            assert_eq!(atlas.alloc(64, 8).map(|slot| slot.y), Some(at * 8));
        }
        atlas.next_frame();
        // Two of them are drawn this frame; the tallest idle run between them is four shelves.
        atlas.touch(1);
        atlas.touch(6);
        assert!(atlas.alloc(10, 40).is_none(), "no five idle neighbours");
        let tall = atlas.alloc(10, 20).expect("three idle neighbours make room");
        assert_eq!((tall.y, tall.evicted.as_slice()), (16, &[2, 3, 4][..]), "24 texels from the third shelf down");
        // The joined shelf is a shelf like any other: more of its height goes beside the first item.
        assert_eq!(atlas.alloc(10, 24).map(|slot| (slot.x, slot.y, slot.shelf)), Some((10, 16, tall.shelf)));
        // A low item still takes one idle low shelf, the least recently used.
        let low = atlas.alloc(64, 8).expect("an idle shelf");
        assert_eq!((low.y, low.evicted.as_slice()), (0, &[0][..]));
        // With the joined shelf in use, the next tall item joins others: the first two, then the last three.
        atlas.next_frame();
        atlas.touch(tall.shelf);
        let first = atlas.alloc(64, 12).expect("two idle neighbours");
        assert_eq!((first.y, first.evicted.as_slice()), (0, &[0, 1][..]));
        let last = atlas.alloc(64, 20).expect("three idle neighbours");
        assert_eq!((last.y, last.evicted.as_slice()), (40, &[5, 6, 7][..]));
        assert!(atlas.alloc(64, 8).is_none(), "everything is in use now");
    }

    #[test]
    fn heights_round_to_buckets() {
        let mut atlas = Shelves::new(64);
        let a = atlas.alloc(10, 9).unwrap();
        let b = atlas.alloc(10, 14).unwrap();
        assert_eq!(a.shelf, b.shelf, "9 and 14 share the 16 bucket");
        let c = atlas.alloc(10, 17).unwrap();
        assert_eq!((c.shelf, c.y), (1, 16));
    }
}
