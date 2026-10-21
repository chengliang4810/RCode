//! Project-scoped HTTP discovery and stable local routing. Dev servers on this
//! device are observed, named and served over one loopback proxy; nothing here
//! leaves the machine.
pub mod catalog;
pub mod discovery;
pub mod mux;
pub mod proxy;

pub mod service;
pub use service::PreviewService;
