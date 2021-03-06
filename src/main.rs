// This example assumes that test items are defined in the dictionary in the following format.
// {
//    "tests": "itemcount, buttonsize",
//    "itemcount": {
//        "buckets": [ "10", "15" ],
//        "weight": "1:1"
//    },
//    "buttonsize": {
//        "buckets": [ "small", "medium", "large" ],
//        "weight": "7:3:2"
//    }
//    "enrich": {
//        "buckets": ["A", "B"],
//        "weight": "1:1"
//    }
//}
mod enrich;

use fastly::http::header::{ACCEPT_ENCODING, CACHE_CONTROL, HOST, SET_COOKIE};
use fastly::{Dictionary, Error, Request, Response};
use rand::distributions::WeightedIndex;
use rand::prelude::*;
use rand::rngs::StdRng;
use serde::{de, Deserialize, Deserializer};
use std::collections::HashMap;
use uuid::Uuid;

const BACKEND_NAME: &str = "origin_0";
const DICT_NAME: &str = "ab_config";
const CID_COOKIE: &str = "ab_cid";

#[derive(Debug, Deserialize)]
struct ABTest {
    buckets: Vec<String>,
    #[serde(deserialize_with = "weight_deserializer")]
    weight: Vec<i32>,
}

// Custom deserializer to parse a weight ratio expression like "7:3:2" into Vec<i32>
fn weight_deserializer<'de, D>(deserializer: D) -> Result<Vec<i32>, D::Error>
where
    D: Deserializer<'de>,
{
    let weight_string = String::deserialize(deserializer)?;
    let mut weights = vec![];
    for w in weight_string.split(":") {
        weights.push(w.parse::<i32>().map_err(|e| {
            de::Error::custom(format!(
                "str::parse::<i32> returned an error while parsing {}: {}",
                weight_string, e
            ))
        })?)
    }
    Ok(weights)
}

struct ClientID {
    id: String,
    is_new: bool,
}

impl ClientID {
    fn new() -> Self {
        Self {
            //id: "newly generated userid".to_string(),
            id: Uuid::new_v4().to_string(),
            is_new: true,
        }
    }
    fn from_id(id: String) -> Self {
        Self { id, is_new: false }
    }
    fn as_setcookie(&self) -> String {
        format!(
            // "{}={}; max-age=31536000; path=/; httponly",
            "{}={}; Max-Age=31536000; Path=/; Secure; HttpOnly",
            CID_COOKIE,
            self.id
        )
    }
}

fn load_cookie(cookie: &str) -> HashMap<String, String> {
    cookie
        .split(";")
        .filter_map(|kv| {
            kv.find("=").map(|index| {
                let (key, value) = kv.split_at(index);
                let key = key.trim().to_string();
                let value = value[1..].to_string();
                (key, value)
            })
        })
        .collect()
}

fn stringify_cookie(cookie_jar: HashMap<String, String>) -> String {
    cookie_jar
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("; ")
}

fn create_rng(cid: &str, test_name: &str) -> StdRng {
    // Mapping a user to the same set of A/B test buckets
    // by generating a seed from a client ID and a test name.
    let digest1: [u8; 16] = md5::compute(cid).into();
    let digest2: [u8; 16] = md5::compute(test_name).into();

    let mut seed: [u8; 32] = Default::default();
    seed[..16].copy_from_slice(&digest1);
    seed[16..].copy_from_slice(&digest2);

    rand::SeedableRng::from_seed(seed)
}

#[fastly::main]
fn main(mut req: Request) -> Result<Response, Error> {
    let abtest_config = Dictionary::open(DICT_NAME);
    if let Some(t) = abtest_config.get("tests") {
        let tests: Vec<String> = t.split(",").map(|t| t.trim().to_string()).collect();

        // Find a client ID and remove it from the origin request
        // so that the origin will not gnerate different content based on the ID.
        // Allocate a client ID if they don't already have one.
        let cid = match req.get_header("cookie") {
            Some(cookie) => {
                let mut cookie_jar = load_cookie(cookie.to_str()?);
                match cookie_jar.remove(CID_COOKIE) {
                    Some(id) => {
                        req.set_header("cookie", stringify_cookie(cookie_jar));
                        ClientID::from_id(id)
                    }
                    None => ClientID::new(),
                }
            }
            None => ClientID::new(),
        };

        // Assign them a bucket for each test and add Fastly-ABTest-X headers to the origin request.
        for test_name in tests {
            match abtest_config.get(&test_name) {
                Some(v) => {
                    let abtest = serde_json::from_str::<ABTest>(&v).unwrap();
                    let mut rng = create_rng(&cid.id, &test_name);

                    // Pick a bucket according to the weight.
                    let dist = WeightedIndex::new(&abtest.weight).unwrap();
                    let bucket = &abtest.buckets[dist.sample(&mut rng)];

                    req.set_header(format!("Fastly-ABTest-{}", test_name), bucket);
                }
                None => {
                    eprintln!(
                        "{} is not found in the dictionary. Sending the request as-is.",
                        test_name
                    );
                    return Ok(req.send(BACKEND_NAME)?);
                }
            }
        }

        // Add the host header so that we don't need to specify it in a request when testing locally
        req.set_header(HOST, "example.com");
        // Request an uncompressed response
        req.remove_header(ACCEPT_ENCODING);
        let mut beresp = req.send(BACKEND_NAME)?;

        if cid.is_new {
            beresp.set_header(SET_COOKIE, cid.as_setcookie());
            beresp.set_header(CACHE_CONTROL, "no-store");
        }

        // If the client is assigned bucket B in the "enrich" test, rewrite the HTML
        match beresp
            .get_backend_request()
            .unwrap()
            .get_header("Fastly-ABTest-enrich")
        {
            Some(bucket) if bucket == "B" => {
                return Ok(enrich::rewrite_html(beresp));
            }
            _ => {
                return Ok(beresp);
            }
        }
    }

    Ok(req.send(BACKEND_NAME)?)
}
