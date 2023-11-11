use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::SystemTime;
use serde::{Deserialize, Serialize};
use warp::Filter;
// use warp::reply::Response;
use sha3::{Digest, Sha3_224};
use base64::{engine::general_purpose, Engine as _};

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Post {
    content: String, // underlying content -- either json or encrypted json
    timestamp: u64, // nanos since epoch
    id: u64 // sequential id. technically this is a "u52" because of float64
}

#[derive(Clone)]
struct ServerData {
    posts: Vec<Post>,
    slice_cache: HashMap<(u32, u32), (String, String)>, // user asks for posts 0-10 (measured from most recent). take that slice, json it, then spit it out.
    posts_json: String,
    current_id: u64,
    posts_hash_b64: String // base64. theoretically i could instead do a "post version" which is just how many /writes have happened, but this seems fragile
}

#[tokio::main]
async fn main() {
    const POSTS_FILENAME: &str = "posts.json";
    let posts_str = match std::fs::read_to_string(POSTS_FILENAME) {
        Ok(val) => val,
        Err(_e) => {
            std::fs::write(POSTS_FILENAME, "[]").unwrap();
            "[]".to_string()
        }
    };
    let posts: Vec<Post> = serde_json::from_str::<Vec<Post>>(&posts_str).unwrap();
    let max_id = match (&posts).into_iter().map(|post| post.id).max() {
        Some(val) => val + 1,
        None => 0
    };
    let server_data = Arc::new(RwLock::new(ServerData{ posts_hash_b64: general_purpose::STANDARD.encode(&Sha3_224::digest(&posts_str)), posts, posts_json: posts_str, current_id: max_id, slice_cache: HashMap::new()}));
    let server_data_2 = server_data.clone();

    let cors = warp::cors()
    .allow_methods(vec!["GET", "POST"])
    .allow_header("Cache").expose_header("Cache")
    .max_age(10000000)
    .allow_any_origin();

    let read = warp::path!("api" / "read").and(warp::header::<String>("Cache")).map(move |cache: String| {
        let server_read = server_data.read().unwrap();
        let resp = warp::http::Response::builder().header("Cache", server_read.posts_hash_b64.clone());
        if server_read.posts_hash_b64 == cache {
            return resp.status(warp::http::StatusCode::NOT_MODIFIED).body("".to_string()).unwrap();
        }
        resp.body(server_read.posts_json.clone()).unwrap()
    });

    let write = warp::path!("api" / "write").and(warp::body::content_length_limit(1024 * 32)).and(warp::body::bytes()).map(move |bytes: bytes::Bytes| {
        let content = match String::from_utf8(bytes.to_vec()) {
            Ok(val) => val,
            Err(_e) => return warp::http::Response::builder().status(warp::http::StatusCode::UNPROCESSABLE_ENTITY).body("input should be utf-8".to_string()).unwrap()
        };
        // would be more efficient to use atomic or something here, but probably fine.
        let mut data = server_data_2.write().unwrap();
        let duration_since_epoch = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap();
        let timestamp = duration_since_epoch.as_millis() as u64;
        let id = data.current_id;
        data.posts.push(Post{content, timestamp, id});
        data.current_id += 1;
        let json_str = serde_json::to_string(&data.posts).unwrap();

        // TODO: spawn a separate thread to do this
        std::fs::write(POSTS_FILENAME, &json_str).unwrap();

        data.posts_json = json_str.clone();
        data.posts_hash_b64 = general_purpose::STANDARD.encode(&Sha3_224::digest(json_str));

        warp::http::Response::builder().header("Cache", data.posts_hash_b64.clone()).body(data.posts_json.clone()).unwrap()
    });

    let options = warp::options().map(warp::reply);

    warp::serve(options.or(read).or(write).with(cors))
        .run(([127, 0, 0, 1], 3030))
        .await;
}
