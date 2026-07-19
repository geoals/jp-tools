pub struct Config {
    pub db_path: String,
    pub listen_addr: String,
}

impl Config {
    pub fn from_env() -> Self {
        let db_path = std::env::var("JP_TOOLS_STATS_DB_PATH").unwrap_or_else(|_| {
            let home = std::env::var("HOME").expect("HOME not set");
            format!("{home}/.local/share/jp-stats/stats.db")
        });
        let listen_addr = std::env::var("JP_TOOLS_STATS_LISTEN_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:3200".to_string());
        Config {
            db_path,
            listen_addr,
        }
    }
}
