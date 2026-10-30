//! Console build of clp2cfg: always headless, prints results and returns an exit code.

fn main() -> std::process::ExitCode {
    clp2cfg::entry::cli_main()
}
