//! This crate builds on top of the struct-patch crate, providing Footnote.
//!
//! Deriving Footnote will generate a `get_note` method which emits an
//! object with the same fields as the original, but stores hashes.
//!
//! The note can be used to check for changed fields or to create a
//! patch instance based on changes from the note.
//!
//! Because of portability issues inherent in Rust's enums,
//! fields must be Serializable rather than Hashable.
//!
//! Fields that also derive Footnote can be nested using `#[patch(nesting)]`.
//!
//! As with struct-patch, individual entries of collections are not handled
//! independently. An entire collection is treated as a single patch item.

pub use footnote_macro::Footnote;

pub trait Footnote {
    type Note: std::hash::Hash + Eq;
    type Patch;
    type Field;

    fn calc_hash<T: serde::ser::Serialize>(t: &T) -> u64 {
        // Silly workaround for the issue that
        // hashing Option & enums not portable between wasm and native.
        // However, bounds fields by Serialize instead of Hash.
        use std::hash::Hasher;
        let mut s = seahash::SeaHasher::new();
        rmp_serde::encode::write_named(&mut s, t).unwrap();
        s.finish()
    }

    fn get_note(&self) -> Self::Note;

    fn check_note(&self, note: &Self::Note) -> Vec<Self::Field>;

    fn make_patch(&self, note: &Self::Note) -> Self::Patch;
}

// Inspect derives with: `cargo expand --package footnote --lib -- demo`
mod demo {
    use super::*;
    use struct_patch::Patch;

    #[allow(unused)]
    #[derive(
        Clone,
        Debug,
        Default,
        PartialEq,
        Eq,
        Hash,
        serde::Serialize,
        serde::Deserialize,
        Patch,
        Footnote,
    )]
    #[patch(attribute(derive(Clone, Debug, Default, PartialEq, Eq, Hash)))]
    pub struct Dummy {
        one: Vec<u8>,

        two: usize,

        #[allow(unused)]
        #[patch(skip)]
        three: bool,
    }

    #[allow(unused)]
    #[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Patch, Footnote)]
    #[patch(attribute(derive(Clone, Debug, Default, PartialEq, Eq, Hash)))]
    pub struct Dumber {
        a: u64,

        // Nest patches and notes so `DumberNote::b` is a `DummyNote`
        #[patch(nesting)]
        b: Dummy,
    }

    #[allow(unused)]
    #[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Patch, Footnote)]
    #[patch(attribute(derive(Clone, Debug, Default, PartialEq, Eq, Hash)))]
    pub struct Dumbest {
        a: u64,

        // Entire `b` field is hashed and patched
        b: Dummy,
    }

    #[allow(unused)]
    #[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Patch, Footnote)]
    #[patch(attribute(derive(Clone, Debug, Default, PartialEq, Eq, Hash)))]
    pub struct Clopping {
        #[note(copied)]
        a: u64,

        // DummyNote::b is just a copy of Dummy, without hashing
        #[note(cloned)]
        b: Dummy,
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_footnote() {
            let mut dummy = Dummy {
                one: vec![1, 2, 3],
                two: 64,
                three: true,
            };

            let hashes = dummy.get_note();
            dummy.two = 66;
            dummy.three = false;

            let changes = dummy.check_note(&hashes);
            assert_eq!(changes, vec![DummyField::Two]);

            let delta = dummy.make_patch(&hashes);
            assert_eq!(delta.one, None);
            assert_eq!(delta.two, Some(66));
        }

        #[test]
        fn test_nest() {
            let dummy = Dummy {
                one: vec![1, 2, 3],
                two: 64,
                three: true,
            };

            let mut dumber = Dumber { a: 24, b: dummy };

            let hashes = dumber.get_note();
            // println!("{hashes:?}");
            dumber.a = 7;
            dumber.b.two = 66;

            let changes = dumber.check_note(&hashes);
            assert_eq!(changes, vec![DumberField::A, DumberField::B]);

            let delta = dumber.make_patch(&hashes);
            // println!("Delta {delta:?}");
            assert_eq!(delta.a, Some(7));
            assert_eq!(delta.b.one, None);
            assert_eq!(delta.b.two, Some(66));
        }

        #[test]
        fn test_blob() {
            let dummy = Dummy {
                one: vec![1, 2, 3],
                two: 64,
                three: true,
            };

            let mut dumbest = Dumbest {
                a: 24,
                b: dummy.clone(),
            };

            let hashes = dumbest.get_note();
            // println!("{hashes:?}");
            dumbest.a = 7;
            dumbest.b.two = 66;

            let changes = dumbest.check_note(&hashes);
            assert_eq!(changes, vec![DumbestField::A, DumbestField::B]);

            let delta = dumbest.make_patch(&hashes);
            // println!("Delta {delta:?}");
            assert_eq!(delta.a, Some(7));
            assert_eq!(delta.b, Some(Dummy { two: 66, ..dummy }));
        }

        #[test]
        fn test_clop() {
            let dummy = Dummy {
                one: vec![1, 2, 3],
                two: 64,
                three: true,
            };

            let mut clop = Clopping {
                a: 24,
                b: dummy.clone(),
            };

            let hashes = clop.get_note();

            assert_eq!(clop.a, hashes.a);
            assert_eq!(clop.b, hashes.b);

            // println!("{hashes:?}");
            clop.a = 7;
            clop.b.two = 66;

            let delta = clop.make_patch(&hashes);
            // println!("Delta {delta:?}");
            assert_eq!(delta.a, Some(7));
            assert_eq!(delta.b, Some(Dummy { two: 66, ..dummy }));
        }
    }
}
