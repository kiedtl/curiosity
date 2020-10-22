use url::{Url, ParseError};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use gemtext::*;
use serde::Serialize;

use std::sync::Arc;
use std::error::Error;
use std::collections::HashMap;
use std::fs;

// stop crawling after encountering MAX urls
const MAX: usize = 1_000_000;

#[derive(Clone, Debug, Serialize)]
struct UrlInfo {
    visited: bool,
    found: usize,
    refers: Vec<String>,
}

impl UrlInfo {
    pub fn new(_ref: String) -> Self {
        Self {
            visited: true,
            found: 1,
            refers: vec![_ref],
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = if std::env::args().count() < 2 {
        String::from("results.json")
    } else {
        std::env::args().collect::<Vec<_>>()[1].clone()
    };

    let urlstr = "gemini://gemini.circumlunar.space:1965/";

    let mut cfg = tokio_rustls::rustls::ClientConfig::new();
    cfg
        .dangerous()
        .set_certificate_verifier(Arc::new(NoCertificateVerification {}));


    let visited = crawl(urlstr, cfg)?;
    fs::write(&output, serde_json::to_string(&visited)?.as_bytes())?;

    println!("\nstored capsule data in {}", output);

    Ok(())
}

fn status(queue_size: usize, visited_size: usize, current_harvest: usize)
{
    print!("\r{q:>0$} queued, {v:>0$} visited     ({ch} now)",
        6, q = queue_size, v = visited_size, ch = current_harvest);
}

fn crawl(start: &str, cfg: tokio_rustls::rustls::ClientConfig)
    -> Result<HashMap<String, UrlInfo>, Box<dyn Error>>
{
    let start = parse_url(None, start)?;

    // map of visited urls
    let mut visited: HashMap<String, UrlInfo> = HashMap::new();

    // queue to visit
    let mut queue: Vec<Url> = Vec::new();

    // start crawling with the first url
    let response = smol::run(get(&start, cfg.clone()))?;
    let urls = extract(&start, response);

    for url in &urls {
        queue.push(url.clone());
        visited.insert(url.to_string(), UrlInfo::new(start.to_string()));
    }

    // main crawl
    while queue.len() > 0 && visited.len() < MAX {
        let link = queue.pop().unwrap();

        let response = smol::run(get(&link, cfg.clone()))?;
        let urls = extract(&link, response);

        for url in &urls {
            if !visited.contains_key(&url.to_string()) {
                queue.push(url.clone());
                visited.insert(url.to_string(), UrlInfo::new(link.to_string()));

                if visited.len() >= MAX {
                    break;
                }
            } else {
                let mut info = visited.get_mut(&url.to_string()).unwrap();
                info.found += 1;
                info.refers.push(link.to_string());
            }

            status(queue.len(), visited.len(), urls.len());
        }
    }

    Ok(visited)
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
        ur.set_port(Some(1965)).unwrap();
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
    let host = ur.host_str().unwrap();
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
