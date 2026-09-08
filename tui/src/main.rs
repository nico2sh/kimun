//! The `kimun` binary is a shim: everything, the app loop included, lives in
//! the `kimun_notes` library so it compiles once and is testable from
//! `tests/`. See `app::main` for the entry.

fn main() -> color_eyre::Result<()> {
    kimun_notes::app::main()
}
