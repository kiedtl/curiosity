use url::{Url, ParseError};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use gemtext::*;
use serde::{Deserialize, Serialize};

use std::sync::Arc;
use std::error::Error;
use std::collections::HashMap;
use std::time::Duration;
use std::fs;

const MAX: usize = 1_000_000;
const TIMEOUT_MS: u64 = 20_000;
const SAVEFREQ: usize = 5000;

const START_URL: &'static str = "gemini://gemini.circumlunar.space:1965/";
const OUTFILE: &'static str = "results.json";

#[derive(Clone, Debug, Deserialize, Serialize)]
struct UrlInfo {
    found: usize,
    refers: Vec<String>,
}

impl UrlInfo {
    pub fn new(_ref: String) -> Self {
        Self {
            found: 1,
            refers: vec![_ref],
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().collect::<Vec<_>>();
    let mut visited = HashMap::new();

    if args.len() > 1 {
        eprint!("Reading from {}... ", &args[1]);
        let json = fs::read_to_string(&args[1])?;

        eprint!("done\nDeserializing JSON data... ");
        visited = serde_json::from_str(&json)?;
        eprint!("done\n");
    }

    let mut cfg = tokio_rustls::rustls::ClientConfig::new();
    cfg
        .dangerous()
        .set_certificate_verifier(Arc::new(NoCertificateVerification {}));


    smol::run(crawl(visited, START_URL, cfg))?;
    Ok(())
}

fn status(queue_size: usize, visited_size: usize, current_harvest: usize)
{
    print!("\r{q:>0$} queued, {v:>0$} visited     ({ch} now)",
        6, q = queue_size, v = visited_size, ch = current_harvest);
}

async fn crawl(mut visited: HashMap<String, UrlInfo>, start: &str,
    cfg: tokio_rustls::rustls::ClientConfig) -> Result<(), Box<dyn Error>>
{
    use tokio::time::timeout;
    let duration = Duration::from_millis(TIMEOUT_MS);

    //use tokio::sync::oneshot;
    //let (tx, rx) = oneshot::channel();

    let start = parse_url(None, start)?;

    // queue to visit
    let mut queue: Vec<Url> = Vec::new();

    // start crawling with the first url
    let response = get(&start, cfg.clone()).await?;
    let urls = extract(&start, response);

    for url in &urls {
        queue.push(url.clone());
        visited.insert(url.to_string(), UrlInfo::new(start.to_string()));
    }

    // main crawl
    let mut savectr = 0;
    while queue.len() > 0 && visited.len() < MAX {
        // move on to the next link
        let link = queue.pop().unwrap();

        // get gemini text
        let response = match timeout(duration, get(&link, cfg.clone())).await {
            Ok(result) => match result {
                Ok(o) => o,
                Err(e) => {
                    eprintln!("\nfailed to fetch {}: {}", link.to_string(), e);
                    continue;
                },
            },
            Err(_) => {
                eprintln!("\nfailed to fetch {}: timed out", link.to_string());
                continue;
            },
        };

        // ...extract urls, and store them to crawl later
        let urls = extract(&link, response);
        status(queue.len(), visited.len(), urls.len());


        for url in &urls {
            if !visited.contains_key(&url.to_string()) {
                queue.push(url.clone());
                visited.insert(url.to_string(), UrlInfo::new(link.to_string()));
                savectr += 1;

                if visited.len() >= MAX {
                    break;
                }

                if savectr == SAVEFREQ {
                    savectr = 0;
                    fs::write(OUTFILE,
                        serde_json::to_string(&visited)?.as_bytes())?;
                    println!("\nstored capsule data in {}", OUTFILE);
                }
            } else {
                let mut info = visited.get_mut(&url.to_string()).unwrap();
                info.found += 1;
                info.refers.push(link.to_string());
            }
        }
    }

    fs::write(OUTFILE, serde_json::to_string(&visited)?.as_bytes())?;
    println!("\nstored capsule data in {}", OUTFILE);
    Ok(())
}


fn extract(base_url: &Url, data: Vec<u8>) -> Vec<Url> {
    let data_s = data.iter().map(|b| *b as char)
        .collect::<String>();
    let parsed = gemtext::parse(&data_s);

    let mut found = Vec::new();

    for node in parsed {
        match node {
            Node::Link { to, name: _ } => {
                match parse_url(Some(&base_url), to.clone()) {
                    Ok(u) => found.push(u),
                    Err(_) => (),
                }
            },
            _ => (),
        }
    }

    found
}

fn parse_url<T>(base_u: Option<&Url>, u: T) -> Result<Url, Box<dyn Error>>
where
    T: Into<String> + Clone
{
    // try to parse url
    // if it fails because the url is relative, try again using
    // base_u as the base url
    let mut ur = match Url::parse(&u.clone().into()) {
        Ok(u) => u,
        Err(cause) => {
            match cause {
                ParseError::RelativeUrlWithoutBase => {
                    if let Some(base) = base_u {
                        base.join(&u.into())?
                    } else {
                        Err("gave relative url, but no base url")?
                    }
                },
                _ => Err(cause)?,
            }
        },
    };

    if ur.port().is_none() {
        // meh, don't need to unwrap this
        let _ = ur.set_port(Some(1965));
    }

    if ur.scheme() != "gemini" {
        Err("invalid url scheme")?;
    }

    Ok(ur)
}

// the following was stolen from Christine Dodrill's majc project
// https://tulpa.dev/cadey/maj
struct NoCertificateVerification {}

impl rustls::ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _roots: &rustls::RootCertStore,
        _presented_certs: &[rustls::Certificate],
        _dns_name: webpki::DNSNameRef<'_>,
        _ocsp: &[u8],
    ) -> Result<rustls::ServerCertVerified, rustls::TLSError> {
        Ok(rustls::ServerCertVerified::assertion())
    }
}

async fn get(ur: &Url, cfg: tokio_rustls::rustls::ClientConfig)
    -> Result<Vec<u8>, Box<dyn std::error::Error>>
{
    use tokio::io::{AsyncWriteExt, AsyncReadExt};

    let cfg = Arc::new(cfg);
    let host = match ur.host_str() {
        Some(h) => h,
        None => return Err("url's host str == None")?,
    };

    let name_ref = webpki::DNSNameRef::try_from_ascii_str(host)?;
    let config = TlsConnector::from(cfg);

    let sock = TcpStream::connect(&format!("{}:{}", host,
            ur.port().unwrap())).await?;
    let mut tls = config.connect(name_ref, sock).await?;

    let req = format!("{}\r\n", ur.to_string());

    tls.write(req.as_bytes()).await?;
    let mut buf: Vec<u8> = vec![];
    tls.read_to_end(&mut buf).await?;

    Ok(buf)
}
