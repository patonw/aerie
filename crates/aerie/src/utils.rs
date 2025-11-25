use std::{borrow::Cow, sync::Arc};

use arc_swap::ArcSwap;
use rpds::{List, ListSync};

pub trait CowExt<'a, T: Clone, E> {
    /// Flat map for cows.
    ///
    /// This is needed when composing functions that return Cow, since an intermediate call might
    /// produce an Owned while the final call returns a Borrowed to it. Since you can't return a
    /// reference to local data, a naive implementation will fail to compile. However, with this
    /// method, after the first Owned is produced, all subsequent calls will result in Owned. The
    /// only way a Borrowed can be the end result is if all calls in the chain borrow the original
    /// value.
    fn moo<F>(&self, f: F) -> Cow<'a, T>
    where
        F: FnOnce(&'_ T) -> Cow<'_, T>;

    /// Flat map for cows, bubbling up error results
    ///
    /// This is needed when composing functions that return Cow, since an intermediate call might
    /// produce an Owned while the final call returns a Borrowed to it. Since you can't return a
    /// reference to local data, a naive implementation will fail to compile. However, with this
    /// method, after the first Owned is produced, all subsequent calls will result in Owned. The
    /// only way a Borrowed can be the end result is if all calls in the chain borrow the original
    /// value.
    fn try_moo<F>(&self, f: F) -> Result<Cow<'a, T>, E>
    where
        F: FnOnce(&'_ T) -> Result<Cow<'_, T>, E>;
}

impl<'a, T: Clone, E> CowExt<'a, T, E> for Cow<'a, T> {
    fn moo<F>(&self, f: F) -> Cow<'a, T>
    where
        F: FnOnce(&'_ T) -> Cow<'_, T>,
    {
        match f(self.as_ref()) {
            Cow::Borrowed(_) => self.clone(),
            Cow::Owned(res) => Cow::Owned(res),
        }
    }

    fn try_moo<F>(&self, f: F) -> Result<Cow<'a, T>, E>
    where
        F: FnOnce(&'_ T) -> Result<Cow<'_, T>, E>,
    {
        Ok(match f(self.as_ref())? {
            Cow::Borrowed(_) => self.clone(),
            Cow::Owned(res) => Cow::Owned(res),
        })
    }
}

// Elements needs to be clonable since rcu may retry to preserve consistency.
// Hence we wrap errors in Arc
pub type ErrorList<E> = Arc<ArcSwap<ListSync<Arc<E>>>>;

pub fn new_errlist<E>() -> ErrorList<E> {
    Arc::new(ArcSwap::from_pointee(rpds::List::new_sync()))
}

/// Trait to queue non-critical errors into a central collection for later inspection
pub trait ErrorDistiller<E> {
    fn discard(&self);

    fn push(&self, err: E);

    /// Diverts errors into a sink while converting result into an option
    fn distil<T>(&self, result: Result<T, E>) -> Option<T> {
        match result {
            Ok(item) => Some(item),
            Err(err) => {
                self.push(err);
                None
            }
        }
    }
}

impl<E> ErrorDistiller<E> for ErrorList<E> {
    fn discard(&self) {
        self.store(Arc::new(List::new_sync()));
    }

    fn push(&self, err: E) {
        let err = Arc::new(err);
        self.rcu(|list| list.push_front(err.clone()));
    }
}
