//! chastitube: YouTube search, videos and playlists, with no feed.
mod links;
mod open;
mod pages;
mod player;
mod server;
mod setup;
mod store;
mod tabs;
mod tags;
mod util;
mod youtube;
mod yt;

const USAGE: &str = "chastitube              open the page (starting the server if needed)
chastitube serve        run the page on http://chastitube (or http://localhost:8790)
chastitube open URL...  link handler: YouTube links open in chastitube;
                        everything else goes to Firefox
chastitube firefox-policy [file]
                        (with sudo) let Firefox open `chastitube` as a site";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("serve") => server::serve(),
        Some("open") => open::open_links(&args[2..]),
        Some("firefox-policy") => setup::firefox_policy(args.get(2).map(String::as_str)),
        None => open::open_app(),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(1);
        }
    }
}
