// Library entry-point exposing modules needed by integration tests.
pub mod ask;
pub mod cli;
pub mod components;
pub mod keys;
pub mod rag;
// Self-contained modules with no dependency on the rest of kimün — see adr/0042.
// Each is a former workspace crate, kept extractable: nothing inside them may
// name `crate::` outside their own subtree (enforced in .github/workflows/check.yml).
pub mod ropetext;
pub mod server_client;
pub mod settings;
pub mod update;
pub mod util;

#[cfg(test)]
mod test_support;
