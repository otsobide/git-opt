//! The core of git-opt: what a repository, a run or an optimization *is*, the
//! rules that apply to them, and the ports (traits) that the outside world
//! implements. Nothing here performs I/O or knows about a framework.

pub mod error;
pub mod fork_cleanup;
pub mod model;
pub mod ports;
pub mod report;
pub mod values;

pub use error::InvalidValue;
