//! The part of clp2cfg that knows nothing about windows, command lines or the clipboard:
//! the domain types, snippet parsing, merging, client detection, and applying to the clients.

pub mod apply;
pub mod delegate;
pub mod discovery;
pub mod domain;
pub mod merge;
pub mod snippet;
pub mod writer;
