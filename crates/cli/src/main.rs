//! `lumen` — the Lumen CLI.
//!
//!   lumen compile <dashboard.json> [-o out.lumen]   compile a dashboard → DEP
//!   lumen serve                                      run the embedding runtime
//!   lumen seed                                       load TPC-H data into Postgres

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use lumen_artifact::Dep;
use lumen_shared::DashboardDef;
use sqlx::postgres::PgPoolOptions;

const SEED_SQL: &str = include_str!("../../../infra/postgres/seed.sql");

#[derive(Parser)]
#[command(name = "lumen", version, about = "Compiled, embeddable analytics runtime")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Compile a dashboard JSON definition into a `.lumen` execution plan.
    Compile {
        /// Path to the dashboard JSON.
        input: PathBuf,
        /// Output `.lumen` path (default: .lumen-build/<id>.lumen).
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Run the embedding runtime (reads config from the environment).
    Serve,
    /// Load TPC-H sample data into Postgres.
    Seed {
        /// Postgres URL (falls back to $DATABASE_URL).
        #[arg(long, env = "DATABASE_URL")]
        database_url: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    match Cli::parse().cmd {
        Cmd::Compile { input, output } => compile_cmd(input, output),
        Cmd::Serve => lumen_runtime::serve(lumen_runtime::Config::from_env())
            .await
            .context("runtime failed"),
        Cmd::Seed { database_url } => seed_cmd(database_url).await,
    }
}

fn compile_cmd(input: PathBuf, output: Option<PathBuf>) -> anyhow::Result<()> {
    let raw = std::fs::read_to_string(&input)
        .with_context(|| format!("reading {}", input.display()))?;
    let mut def: DashboardDef =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", input.display()))?;

    if def.id.is_none() {
        def.id = input
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string);
    }
    let id = def.id.clone().unwrap_or_else(|| "dashboard".into());
    let out = output.unwrap_or_else(|| PathBuf::from(".lumen-build").join(format!("{id}.lumen")));

    let n = lumen_compiler::compile_to_file(&def, &out)?;

    // Reload + summarize so the user sees what the compiler produced.
    let bytes = std::fs::read(&out)?;
    let dep = Dep::from_bytes(bytes)?;
    let m = dep.manifest()?;
    println!("✓ compiled {} → {} ({} bytes)", input.display(), out.display(), n);
    println!(
        "  {} · {} widgets · {} distinct queries · ~{} credits · renderers: {:?}",
        m.title,
        m.widgets.len(),
        m.queries.len(),
        m.required_credits.estimate,
        m.renderers,
    );
    println!("  content hash: {}", m.content_hash);
    Ok(())
}

async fn seed_cmd(database_url: Option<String>) -> anyhow::Result<()> {
    let url = database_url
        .or_else(|| std::env::var("DATABASE_URL").ok())
        .context("no DATABASE_URL provided (pass --database-url or set $DATABASE_URL)")?;

    println!("connecting to {url} …");
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .context("connecting to Postgres")?;

    println!("seeding TPC-H sample data (this drops & recreates the tables) …");
    sqlx::raw_sql(SEED_SQL)
        .execute(&pool)
        .await
        .context("running seed.sql")?;

    let orders: i64 = sqlx::query_scalar("SELECT count(*) FROM orders")
        .fetch_one(&pool)
        .await?;
    let lineitems: i64 = sqlx::query_scalar("SELECT count(*) FROM lineitem")
        .fetch_one(&pool)
        .await?;
    let revenue: Option<f64> = sqlx::query_scalar("SELECT sum(o_totalprice)::float8 FROM orders")
        .fetch_one(&pool)
        .await?;
    println!(
        "✓ seeded: {orders} orders, {lineitems} lineitems, total order value ${:.0}",
        revenue.unwrap_or(0.0)
    );
    Ok(())
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("lumen=info,lumen_runtime=info,info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
