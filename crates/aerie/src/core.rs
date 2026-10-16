use std::hash::{DefaultHasher, Hash, Hasher};

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Id(u64);

impl Id {
    pub fn new(value: impl Hash) -> Self {
        let mut s = DefaultHasher::new();
        value.hash(&mut s);
        Self(s.finish())
    }

    pub fn with(&self, child: impl Hash) -> Self {
        let mut s = DefaultHasher::new();
        s.write_u64(self.0);
        child.hash(&mut s);
        Self(s.finish())
    }
}
