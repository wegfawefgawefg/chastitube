//! chastitube: YouTube search, videos and playlists, with no feed.
mod links;
mod open;
mod pages;
mod player;
mod server;
mod store;
mod tabs;
mod tags;
mod util;
mod youtube;
mod yt;

const USAGE: &str = "chastitube serve        run the local page on http://localhost:8790
chastitube open URL...  link handler: YouTube links open in chastitube;
                        everything else goes to Firefox";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("serve") => server::serve(),
        Some("open") => open::open_links(&args[2..]),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(1);
        }
    }
}
