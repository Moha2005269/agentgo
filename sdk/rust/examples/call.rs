//! cargo run --example call -- <port> <token file> <method> [params JSON]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let client = agentgo::Client::from_token_file(args[1].parse().expect("port"), &args[2]).expect("token");
    let params = args.get(4).map(|p| serde_json::from_str(p).expect("params JSON")).unwrap_or(serde_json::json!({}));
    match client.call(&args[3], params) {
        Ok(result) => println!("{result}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
