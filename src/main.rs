use actix_web::{
    client::{Client, ClientBuilder},
    get,
    http::{header, HeaderValue},
    middleware, post,
    web::{Data, Json},
    App, HttpResponse, HttpServer,
};
use bb8_postgres::{
    tokio_postgres::{config::Config, NoTls},
    PostgresConnectionManager,
};
use bb8_redis::{bb8, redis::AsyncCommands, RedisConnectionManager, RedisPool};
use rand::{prelude::Rng, thread_rng};
use serde::Deserialize;
use serde_json::{from_slice, to_vec, Value};

use std::env::{set_var, var};
use std::io::Result as IoResult;
use std::str::FromStr;

mod id;

type PgPool = Data<bb8::Pool<PostgresConnectionManager<NoTls>>>;

enum CrateRarity {
    Common,
    Uncommon,
    Rare,
    Magic,
    Legendary,
}

impl<'a> CrateRarity {
    fn column_name(&'a self) -> &'a str {
        match self {
            CrateRarity::Common => "crates_common",
            CrateRarity::Uncommon => "crates_uncommon",
            CrateRarity::Rare => "crates_rare",
            CrateRarity::Magic => "crates_magic",
            CrateRarity::Legendary => "crates_legendary",
        }
    }

    fn name(&'a self) -> &'a str {
        match self {
            CrateRarity::Common => "common",
            CrateRarity::Uncommon => "uncommon",
            CrateRarity::Rare => "rare",
            CrateRarity::Magic => "magic",
            CrateRarity::Legendary => "legendary",
        }
    }
}

#[derive(Deserialize)]
struct TopGGRequest {
    user: id::UserId,
}

#[derive(Deserialize)]
struct DblRequest {
    id: id::UserId,
}

#[post("/topgg")]
async fn top_gg(
    req: Json<TopGGRequest>,
    redis_pool: Data<RedisPool>,
    pg_pool: PgPool,
    session: Data<Client>,
) -> HttpResponse {
    let user = req.user.0;
    handle_vote(redis_pool, pg_pool, session, user, "topgg-vote", 43200).await;
    HttpResponse::Ok().finish()
}

#[post("/dbl")]
async fn dbl(
    req: Json<DblRequest>,
    redis_pool: Data<RedisPool>,
    pg_pool: PgPool,
    session: Data<Client>,
) -> HttpResponse {
    let user = req.id.0;
    handle_vote(redis_pool, pg_pool, session, user, "dbl-vote", 86400).await;
    HttpResponse::Ok().finish()
}

async fn handle_vote(
    redis_pool: Data<RedisPool>,
    pg_pool: PgPool,
    session: Data<Client>,
    user: i64,
    redis_key: &str,
    timer: usize,
) {
    let mut redis_conn = redis_pool.get().await.unwrap();
    let redis_conn = redis_conn.as_mut().unwrap();
    let pg_conn = pg_pool.get().await.unwrap();

    let mut rng = thread_rng();
    let r: i32 = rng.gen_range(0, 10001);

    let rarity = match r {
        0..=10 => CrateRarity::Legendary,
        11..=100 => CrateRarity::Magic,
        101..=500 => CrateRarity::Rare,
        501..=1000 => CrateRarity::Uncommon,
        _ => CrateRarity::Common,
    };
    let rarity_string = rarity.column_name();
    let rarity_name = rarity.name();

    pg_conn
        .execute(
            &*format!(
                "UPDATE profile SET {0:?}={0:?}+1 WHERE \"user\"=$1;",
                rarity_string
            ),
            &[&user],
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
        cache_parsed[rarity_string] =
            Value::from(cache_parsed[rarity_string].as_u64().unwrap() + 1);
        let cache_new = to_vec(&cache_parsed).unwrap();
        let _: () = redis_conn.set(profile_key, cache_new).await.unwrap();
    }

    // JSON keys identical :P
    let resp: DblRequest = session
        .post("https://discord.com/api/v8/users/@me/channels")
        .send_body(format!("{{\"recipient_id\": \"{}\"}}", user))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    session
        .post(&format!(
            "https://discord.com/api/v8/channels/{}/messages",
            resp.id.0
        ))
        .send_body(format!(
            "{{\"content\":\"Thank you for the upvote! You received a {} crate!\"}}",
            rarity_name
        ))
        .await
        .unwrap();
}

#[get("/")]
async fn metrics() -> HttpResponse {
    HttpResponse::Ok().body("1")
}

#[actix_web::main]
async fn main() -> IoResult<()> {
    set_var("RUST_LOG", "actix_web=debug,actix_server=info");
    env_logger::init();

    let manager = RedisConnectionManager::new("redis://127.0.0.1:6379").unwrap();
    let pool = RedisPool::new(bb8::Pool::builder().build(manager).await.unwrap());

    let pgmanager = PostgresConnectionManager::new(
        Config::from_str(&var("DATABASE_URI").unwrap()).unwrap(),
        NoTls,
    );
    let pgpool = bb8::Pool::builder().build(pgmanager).await.unwrap();

    HttpServer::new(move || {
        App::new()
            .wrap(middleware::Logger::default())
            .data(pool.clone())
            .data(pgpool.clone())
            .data_factory(|| async {
                let token = var("DISCORD_TOKEN").unwrap();
                let client = ClientBuilder::new()
                    .no_default_headers()
                    .header(
                        header::AUTHORIZATION,
                        HeaderValue::from_str(&format!("Bot {}", token)).unwrap(),
                    )
                    .header(
                        header::USER_AGENT,
                        HeaderValue::from_static("DiscordBotVoteHandlerRust (0.1.0) IdleRPG"),
                    )
                    .header(
                        header::CONTENT_TYPE,
                        HeaderValue::from_static("application/json"),
                    )
                    .header(
                        header::ACCEPT_ENCODING,
                        HeaderValue::from_static("identity"),
                    )
                    .finish();
                Ok::<Client, ()>(client)
            })
            .service(top_gg)
            .service(dbl)
            .service(metrics)
    })
    .bind("0.0.0.0:7666")?
    .run()
    .await
}
