use bb8_postgres::{
    tokio_postgres::{config::Config, NoTls},
    PostgresConnectionManager,
};
use bb8_redis::{bb8, redis::AsyncCommands, RedisConnectionManager};
use hyper::{
    body::{aggregate, Buf},
    client::HttpConnector,
    header,
    service::{make_service_fn, service_fn},
    Body, Client, Method, Request, Response, Server, StatusCode,
};
use hyper_rustls::HttpsConnector;
use lazy_static::lazy_static;
use log::{error, info};
use rand::{prelude::Rng, thread_rng};
use serde::Deserialize;
use serde_json::{from_slice, to_vec, Value};

use std::{
    env::{set_var, var},
    io::Result as IoResult,
    net::SocketAddr,
    str::FromStr,
};

type GenericError = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, GenericError>;
type PgPool = bb8::Pool<PostgresConnectionManager<NoTls>>;
type RedisPool = bb8::Pool<RedisConnectionManager>;
static NOTFOUND: &[u8] = b"Not Found";

#[derive(Debug, Deserialize)]
struct TopGGRequest {
    user: String,
}

#[derive(Debug, Deserialize)]
struct DblRequest {
    id: String,
}

async fn handle_vote(
    redis_pool: RedisPool,
    pg_pool: PgPool,
    session: Client<HttpsConnector<HttpConnector>>,
    user: String,
    redis_key: &str,
    timer: usize,
) {
    let mut redis_conn = redis_pool.get().await.unwrap();
    let pg_conn = pg_pool.get().await.unwrap();

    let r = {
        let mut rng = thread_rng();
        rng.gen_range(0..10001)
    };

    let rarity_name = match r {
        0..=10 => "legendary",
        11..=100 => "magic",
        101..=500 => "rare",
        501..=1000 => "uncommon",
        _ => "common",
    };
    let rarity_string = format!("crates_{}", rarity_name);

    pg_conn
        .execute(
            &*format!(
                "UPDATE profile SET {0:?}={0:?}+1 WHERE \"user\"=$1;",
                rarity_string
            ),
            &[&user.parse::<i64>().unwrap()],
        )
        .await
        .unwrap();

    let _: () = redis_conn
        .set_ex(format!("cd:{}:{}", user, redis_key), "vote", timer)
        .await
        .unwrap();

    let profile_key = format!("profilecache:{}", user);
    let cache_data: Vec<u8> = redis_conn.get(&profile_key).await.unwrap();
    if !cache_data.is_empty() {
        let mut cache_parsed: Value = from_slice(&cache_data).unwrap();
        cache_parsed[&rarity_string] =
            Value::from(cache_parsed[&rarity_string].as_u64().unwrap() + 1);
        let cache_new = to_vec(&cache_parsed).unwrap();
        let _: () = redis_conn.set(profile_key, cache_new).await.unwrap();
    }

    let mut req = Request::builder()
        .method("POST")
        .uri("https://discord.com/api/v8/users/@me/channels")
        .body(Body::from(format!("{{\"recipient_id\": \"{}\"}}", user)))
        .unwrap();
    let headers = req.headers_mut();
    headers.extend(HEADERS.clone());

    // JSON keys identical :P
    let resp = session.request(req).await.unwrap();
    let body = aggregate(resp).await.unwrap();
    let data: DblRequest = serde_json::from_reader(body.reader()).unwrap();

    let mut req = Request::builder()
        .method("POST")
        .uri(format!(
            "https://discord.com/api/v8/channels/{}/messages",
            data.id
        ))
        .body(Body::from(format!(
            "{{\"content\":\"Thank you for the upvote! You received a {} crate!\"}}",
            rarity_name
        )))
        .unwrap();
    let headers = req.headers_mut();
    headers.extend(HEADERS.clone());
    session.request(req).await.unwrap();
}

async fn handle(
    req: Request<Body>,
    pg_pool: PgPool,
    redis_pool: RedisPool,
    client: Client<HttpsConnector<HttpConnector>>,
) -> Result<Response<Body>> {
    info!("{} request to {}", req.method(), req.uri().path());

    match (req.method(), req.uri().path()) {
        (&Method::GET, "/") => Ok(Response::new(Body::from("1"))),
        (&Method::POST, "/topgg") => {
            let whole_body = aggregate(req).await?;
            let data: TopGGRequest = serde_json::from_reader(whole_body.reader())?;
            handle_vote(redis_pool, pg_pool, client, data.user, "topgg-vote", 43200).await;
            Ok(Response::new(Body::from("")))
        }
        (&Method::POST, "/dbl") => {
            let whole_body = aggregate(req).await?;
            let data: DblRequest = serde_json::from_reader(whole_body.reader())?;
            handle_vote(redis_pool, pg_pool, client, data.id, "dbl-vote", 43200).await;
            Ok(Response::new(Body::from("")))
        }
        _ => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(NOTFOUND.into())
            .unwrap()),
    }
}

async fn serve(
    req: Request<Body>,
    pg_pool: PgPool,
    redis_pool: RedisPool,
    client: Client<HttpsConnector<HttpConnector>>,
) -> Result<Response<Body>> {
    match handle(req, pg_pool, redis_pool, client).await {
        Ok(resp) => Ok(resp),
        Err(e) => {
            error!("{:?}", e);
            Ok(Response::builder()
                .status(500)
                .body("Internal server error".into())
                .unwrap())
        }
    }
}

lazy_static! {
    static ref HEADERS: header::HeaderMap = {
        let token = var("DISCORD_TOKEN").unwrap();
        let mut map = header::HeaderMap::new();
        map.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&format!("Bot {}", token)).unwrap(),
        );
        map.insert(
            header::USER_AGENT,
            header::HeaderValue::from_static("DiscordBotVoteHandlerRust (0.1.0) IdleRPG"),
        );
        map.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        );
        map
    };
}

#[tokio::main]
async fn main() -> IoResult<()> {
    set_var("RUST_LOG", "info");
    env_logger::init();
    let https = HttpsConnector::with_native_roots();
    let client = Client::builder().build(https);

    let redis_manager = RedisConnectionManager::new("redis://127.0.0.1:6379").unwrap();
    let redis_pool = bb8::Pool::builder().build(redis_manager).await.unwrap();

    let pg_manager = PostgresConnectionManager::new(
        Config::from_str(&var("DATABASE_URI").unwrap()).unwrap(),
        NoTls,
    );
    let pg_pool = bb8::Pool::builder().build(pg_manager).await.unwrap();

    let addr = SocketAddr::from(([0, 0, 0, 0], 7666));
    let make_service = make_service_fn(move |_| {
        let client = client.clone();
        let pg_pool = pg_pool.clone();
        let redis_pool = redis_pool.clone();

        async move {
            Ok::<_, GenericError>(service_fn(move |req| {
                serve(req, pg_pool.clone(), redis_pool.clone(), client.clone())
            }))
        }
    });
    let server = Server::bind(&addr).serve(make_service);

    info!("teatro listening on port 7666");

    if let Err(e) = server.await {
        error!("{:?}", e);
    };

    Ok(())
}
