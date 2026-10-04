//! `capia` — cliente de linha de comando do engine. Não tem parser, engine nem SQL próprios:
//! só traduz argumentos para chamadas à fachada `capia-project` (que usa o Command Engine e o
//! store). Serve para testar e automatizar o engine fora da UI.

mod assets_cmd;
mod cli;
mod media_cmd;
mod render_cmd;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = cli::run(&args, &mut std::io::stdout(), &mut std::io::stderr());
    std::process::exit(code);
}
