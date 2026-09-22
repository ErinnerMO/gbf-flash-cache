use gbf_flash_cache_core::memory::{Admission, MemoryCache};
use gbf_flash_cache_core::{
    refs::ResourceRefs,
    scheduler::Scheduler,
    storage::{variant, Entry},
    Headers, CDN,
};
use serde_json::{json, Value};
use std::io::{self, BufRead};
use std::sync::Arc;
fn text(v: &Value, i: usize) -> &str {
    v[i].as_str().unwrap()
}
fn bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn headers(s: &str) -> Headers {
    if s.is_empty() {
        vec![]
    } else {
        s.split('\n')
            .map(|s| {
                let (n, v) = s.split_once('\t').unwrap();
                (n.into(), v.into())
            })
            .collect()
    }
}
fn main() {
    let mut memory = MemoryCache::new(0);
    let mut parser = ResourceRefs::new(CDN).unwrap();
    let mut scheduler = Scheduler::default();
    let mut scheduler_parser = ResourceRefs::new(CDN).unwrap();
    for line in io::stdin().lock().lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let mut result = Value::Null;
        match text(&v, 0) {
            "reset" => {
                parser = ResourceRefs::new(CDN).unwrap();
                scheduler_parser = ResourceRefs::new(CDN).unwrap();
                scheduler = Scheduler::default();
            }
            "parse" => {
                let refs = parser.parse(text(&v, 1), text(&v, 2), text(&v, 3), &bytes(text(&v, 4)));
                result = json!([parser.reason, refs.into_iter().collect::<Vec<_>>()]);
            }
            "asset" => result = json!(parser.asset(text(&v, 1))),
            "demand" => scheduler.demand(text(&v, 1), text(&v, 2).parse().unwrap()),
            "response" => scheduler.response(text(&v, 1), text(&v, 2) == "true"),
            "discover" => {
                let refs =
                    scheduler_parser.parse(text(&v, 1), text(&v, 2), "", text(&v, 3).as_bytes());
                scheduler.discover(text(&v, 1), &refs, text(&v, 4).parse().unwrap());
            }
            "take" => result = json!(scheduler.take(text(&v, 1).parse().unwrap()).map(|j| j.url)),
            "memory_reset" => memory = MemoryCache::new(text(&v, 1).parse().unwrap()),
            "memory_put" => {
                memory.put(
                    text(&v, 1).into(),
                    Arc::new(Entry {
                        checked: 0,
                        headers: vec![],
                        variant: String::new(),
                        body: bytes(text(&v, 2)),
                    }),
                    Admission::Preload,
                );
                result = json!(memory.bytes());
            }
            "memory_get" => {
                result = json!(memory.get(text(&v, 1), text(&v, 2) == "true").map(|e| e
                    .body
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()))
            }
            "memory_remove" => {
                memory.remove(text(&v, 1));
                result = json!(memory.bytes());
            }
            "memory_close" => {
                memory.close();
                result = json!(memory.bytes());
            }
            "storage" => {
                let response = headers(text(&v, 2));
                let request = headers(text(&v, 3));
                let entry = Entry {
                    checked: text(&v, 1).parse().unwrap(),
                    variant: variant(&response, &request),
                    headers: response,
                    body: bytes(text(&v, 4)),
                };
                let mut out = vec![];
                entry.write(&mut out).unwrap();
                assert_eq!(Entry::read(out.as_slice()).unwrap(), entry);
                assert!(entry.matches(&request));
                result = json!([entry.variant, entry.not_modified(&request)]);
            }
            _ => panic!("unknown operation"),
        }
        println!("{result}");
    }
}
