var express = require("express");
var redis = require("redis");
var Pool = require("pg").Pool;
var { promisify } = require("util");
var fetch = require("node-fetch");

var config = require("./config.json");

var redisClient = redis.createClient();
var setAsync = promisify(redisClient.set).bind(redisClient);
var pool = new Pool({
  user: config.databaseUser,
  host: config.databaseHost,
  database: config.databaseName,
  password: config.databasePassword,
  port: config.databasePort
});
var app = express();
var headers = {
  Authorization: `Bot ${config.token}`,
  "Content-Type": "application/json",
  "User-Agent": "DiscordVoteHandlerJS (0.7.0) IdleRPG"
};
var BASE_URL = "https://discordapp.com/api/v6";

async function getJson(endpoint, data) {
  res = await fetch(BASE_URL + endpoint, data);
  return res.json();
}

app.post("/", async (req, res) => {
  var user = req.data.user;
  if (!user) return res.status(500).send("No user");
  var rand = Math.random();
  if (rand <= 0.001) {
    var rarity = "legendary";
  } else if (rand <= 0.01) {
    var rarity = "magic";
  } else if (rand <= 0.05) {
    var rarity = "rare";
  } else if (rand <= 0.1) {
    var rarity = "uncommon";
  } else {
    var rarity = "common";
  }
  await pool.query(
    `UPDATE profile SET "crates_${rarity}"="crates_${rarity}"+1 WHERE "user"=$1;`,
    [user]
  );
  await setAsync(`cd:${user}:vote`, "vote", "EX", 43200);
  var json = await getJson("users/@me/channels", {
    method: "POST",
    body: JSON.stringify({ recipients: [user] }),
    headers: headers
  });
  var id = json.id;
  await getJson(`channels/${id}/messages`, {
    method: "POST",
    body: JSON.stringify({
      content: `Thank you for the upvote! You received a ${rarity} crate!`,
      nonce: user,
      tts: false
    }),
    headers: headers
  });
});

app.listen(7666, function() {
  console.log("Votehandler running on port 7666");
});
