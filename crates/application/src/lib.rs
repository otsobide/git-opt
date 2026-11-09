//! git-opt's use cases. They only know the domain and its ports: storage,
//! version control, the code forge and format strategies are injected
//! through [`Ports`] and [`StrategyCatalog`].
//!
//! Strategies chosen at run time:
//! - [`FormatStrategy`](git_opt_domain::ports::FormatStrategy) per file format (from the catalog);
//! - [`delivery::DeliveryStrategy`] per repository mode (direct or via fork);
//! - a chain of author sources (settings → VCS config → forge account).

pub mod authorship;
pub mod catalog;
pub mod delivery;
mod error;
pub mod jobs;
pub mod scheduler;
mod services;
pub mod use_cases;

pub use error::{AppError, AppResult};
pub use services::{Ports, Services};

#[cfg(test)]
mod tests;
