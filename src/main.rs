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

const TIMEOUT_MS: u64 = 5000;
const SAVEFREQ: usize = 1000;

const START_URL: &'static str = "gemini://gemini.circumlunar.space:1965/";
const OUTFILE: &'static str = "results.json";

#[derive(Clone, Debug, Deserialize, Serialize)]
struct UrlInfo {
    referred_from: Vec<String>,
    timed_out: bool,
    malformed_response: bool,
    response_code: usize,
    metatext: String,
}

impl UrlInfo {
    pub fn new(_ref: String) -> Self {
        Self {
            referred_from: vec![_ref],
            timed_out: false,
            malformed_response: false,
            response_code: 0,
            metatext: "".to_string(),
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().collect::<Vec<_>>();
    let mut entries = HashMap::new();

    if args.len() > 1 {
        eprint!("Reading from {}... ", &args[1]);
        let json = fs::read_to_string(&args[1])?;

        eprint!("done\nDeserializing JSON data... ");
        entries = serde_json::from_str(&json)?;
        eprint!("done\n");
    }

    let mut cfg = tokio_rustls::rustls::ClientConfig::new();
    cfg
        .dangerous()
        .set_certificate_verifier(Arc::new(NoCertificateVerification {}));


    smol::run(crawl(entries, START_URL, cfg))?;
    Ok(())
}

async fn crawl(mut entries: HashMap<String, UrlInfo>, start: &str,
    cfg: tokio_rustls::rustls::ClientConfig) -> Result<(), Box<dyn Error>>
{
    use tokio::time::timeout;
    let duration = Duration::from_millis(TIMEOUT_MS);

    let start = parse_url(None, start)?;

    // queue to visit
    let mut queue: Vec<Url> = Vec::new();

    // start crawling with the first url
    let response = get(&start, cfg.clone()).await?;
    let urls = extract_urls(&start, response);

    for url in &urls {
        queue.push(url.clone());
        entries.insert(url.to_string(), UrlInfo::new(start.to_string()));
    }

    // main crawl
    let mut savectr = 0;
    while queue.len() > 0 {
        savectr += 1;
        if savectr == SAVEFREQ {
            save_data(&mut entries)?;
            savectr = 0;
        }

        status(queue.len(), entries.len(), 0);

        // move on to the next link
        let link = queue.pop().unwrap();
        let link_str = &link.to_string();
        let mut link_info = entries.get_mut(link_str).unwrap();

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
                link_info.timed_out = true;
                continue;
            },
        };

        if response.len() == 0 {
            continue;
        }

        let response_str = match std::str::from_utf8(&response) {
            Ok(s) => s,
            Err(_) => { link_info.malformed_response = true; continue; },
        };

        let header;
        if let Some(h) = response_str.split("\n").next() {
            header = h;
        } else {
            link_info.malformed_response = true;
            continue;
        }

        let response_code_str = header[0..=1].to_string();
        let response_code = match response_code_str.parse::<usize>() {
            Ok(r) => r,
            Err(_) => { link_info.malformed_response = true; continue; },
        };
        let metatext = header[3..].to_string();

        link_info.response_code = response_code;
        link_info.metatext = metatext.clone();

        match response_code {
            10 => (), // input required
            11 => (), // sensitive input required
            // 20 success
            20 => {
                if metatext.starts_with("text/gemini") {
                    handle_gemtext(&mut entries, &mut queue, &link, response);
                }
            },
            30 => (), // temporary redirect
            31 => (), // permanent redirect
            40 => (), // temporary failure
            41 => (), // server unavailable (load or maintainance)
            42 => (), // cgi/cms error
            43 => (), // proxy error
            44 => (), // slow down (ratelimited)
            50 => (), // permanent failure
            51 => (), // not found
            52 => (), // gone (removed permanently)
            53 => (), // proxy request refused
            59 => (), // malformed request
            60 => (), // client cert required
            61 => (), // unauthorised client cert used
            62 => (), // invalid client cert used
            _ => (),  // ???
        }
    }

    save_data(&mut entries)?;
    Ok(())
}

fn handle_gemtext(
    entries: &mut HashMap<String, UrlInfo>,
    queue: &mut Vec<Url>,
    base_url: &Url,
    data: Vec<u8>
) {
    // ...extract urls, and store them to crawl later
    let urls = extract_urls(&base_url, data);

    for url in &urls {
        status(queue.len(), entries.len(), urls.len());

        if !entries.contains_key(&url.to_string()) {
            queue.push(url.clone());
            entries.insert(url.to_string(), UrlInfo::new(base_url.to_string()));
        } else {
            let info = entries.get_mut(&url.to_string()).unwrap();
            info.referred_from.push(base_url.to_string());
        }
    }
}

fn status(
    queue_size: usize, entries: usize,
    current_harvest: usize,
) {
    print!("\r{q:>0$} queued, {v:>0$} entries     ({ch} now)",
        6, q = queue_size, v = entries, ch = current_harvest);
}

fn save_data(entries: &mut HashMap<String, UrlInfo>) -> Result<(), Box<dyn Error>> {
    fs::write(OUTFILE, serde_json::to_string(&entries)?.as_bytes())?;
    println!("\nstored capsule data in {}", OUTFILE);
    Ok(())
}

fn extract_urls(base_url: &Url, data: Vec<u8>) -> Vec<Url> {
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
