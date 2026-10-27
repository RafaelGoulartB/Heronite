//! The Windows store clients Heronite runs under Wine: Battle.net, the EA
//! app, Ubisoft Connect and the Rockstar Games Launcher (the clients
//! themselves are `heronite_data::clients::Client`).
//!
//! * [`reg`]: Wine's registry files, read without Wine.
//! * [`winpath`]: the prefix's Windows paths as Linux paths.

pub mod reg;
pub mod winpath;
