use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

pub async fn create_pool(database_url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(8)
        .min_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .idle_timeout(std::time::Duration::from_secs(600))
        .connect(database_url)
        .await
        .expect("Failed to connect to PostgreSQL")
}

pub async fn run_migrations(pool: &PgPool) {
    sqlx::raw_sql(include_str!("../migrations/001_initial_schema.sql"))
        .execute(pool)
        .await
        .expect("Failed to run migration 001");

    sqlx::raw_sql(include_str!("../migrations/002_user_cache.sql"))
        .execute(pool)
        .await
        .expect("Failed to run migration 002");

    sqlx::raw_sql(include_str!("../migrations/003_game_universes.sql"))
        .execute(pool)
        .await
        .expect("Failed to run migration 003");

    sqlx::raw_sql(include_str!("../migrations/004_player_game_stats.sql"))
        .execute(pool)
        .await
        .expect("Failed to run migration 004");

    sqlx::raw_sql(include_str!("../migrations/005_game_universes_guild_scope.sql"))
        .execute(pool)
        .await
        .expect("Failed to run migration 005");

    sqlx::raw_sql(include_str!("../migrations/006_linked_accounts_discord_username.sql"))
        .execute(pool)
        .await
        .expect("Failed to run migration 006");

    sqlx::raw_sql(include_str!("../migrations/007_universe_mode.sql"))
        .execute(pool)
        .await
        .expect("Failed to run migration 007");

    sqlx::raw_sql(include_str!("../migrations/008_entry_key_template.sql"))
        .execute(pool)
        .await
        .expect("Failed to run migration 008");
}
