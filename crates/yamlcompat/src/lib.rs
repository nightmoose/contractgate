//! serde_yaml 0.9 stand-in for DataContractGate.
//!
//! The public items are noyalib's serde_yaml compatibility shim, so existing
//! `serde_yaml::from_str` / `Value` / `Mapping` call sites keep working
//! without unmaintained `unsafe-libyaml`.

pub use noyalib::compat::serde_yaml::*;
