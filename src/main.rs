mod browser;
mod cache;
mod cli;
mod config;
mod error;
mod model;
mod output;
mod scraper;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{Cli, Commands, Section, SortOrder};
use config::AppConfig;
use std::time::SystemTime;

use crate::browser::session::BrowserSession;
use crate::cache::Cache;
use crate::error::IherbError;
use crate::scraper::navigation::Navigator;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let json_output = cli.json;

    let exit_code = match run(cli).await {
        Ok(()) => 0,
        Err(err) => {
            let (error_type, exit_code) = classify_error(&err);
            if json_output {
                eprintln!(
                    "{}",
                    output::format_error_json(error_type, &err.to_string())
                );
            } else {
                eprintln!("{}", err);
            }
            exit_code
        }
    };
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
}

async fn run(cli: Cli) -> Result<()> {
    let json_output = cli.json;

    let filter = if cli.debug {
        "iherb_cli=debug"
    } else {
        "iherb_cli=warn"
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();

    let config = AppConfig::load(
        cli.country,
        cli.currency,
        cli.no_cache,
        cli.delay,
        cli.debug,
        cli.profile_dir,
        cli.timing,
    )?;

    ctrlc::set_handler(|| {
        eprintln!("\nInterrupted.");
        std::process::exit(130);
    })
    .context("Failed to set Ctrl+C handler")?;

    let mut browser_session: Option<BrowserSession> = None;

    match cli.command {
        None | Some(Commands::Setup) => {
            cmd_setup(&config, &mut browser_session).await?;
        }
        Some(Commands::Search {
            query,
            limit,
            sort,
            category,
        }) => {
            cmd_search(
                &config,
                &mut browser_session,
                &query,
                limit,
                sort,
                category.as_deref(),
                json_output,
            )
            .await?;
        }
        Some(Commands::Product { id_or_url, section }) => {
            cmd_product(
                &config,
                &mut browser_session,
                &id_or_url,
                section,
                json_output,
            )
            .await?;
        }
    }

    if let Some(session) = browser_session.take() {
        if let Err(e) = session.close().await {
            tracing::warn!("Failed to close browser: {}", e);
        }
    }

    Ok(())
}

async fn cmd_setup(config: &AppConfig, browser_session: &mut Option<BrowserSession>) -> Result<()> {
    let session = get_or_launch_browser(config, browser_session).await?;
    let page = session.new_page().await?;
    let url = config.base_url();

    if config.debug {
        page.goto(&url)
            .await
            .context("Failed to open iHerb homepage for profile setup")?;
        tokio::time::sleep(std::time::Duration::from_millis(config.delay_ms)).await;
    } else {
        let navigator = Navigator::new(config.delay_ms, config.timing);
        navigator
            .navigate_with_retry(&page, &url, 0, scraper::navigation::ReadinessTarget::None)
            .await
            .context("Failed to navigate to iHerb homepage")?;
    }

    if config.debug {
        eprintln!(
            "Opened {}. Complete Cloudflare/login and set US/English in this profile, then press Ctrl+C when done.",
            url
        );
        futures::future::pending::<()>().await;
    } else {
        println!("Opened {}", url);
    }

    Ok(())
}

fn classify_error(err: &anyhow::Error) -> (&'static str, i32) {
    for cause in err.chain() {
        if let Some(iherb) = cause.downcast_ref::<IherbError>() {
            return match iherb {
                IherbError::CloudflareBlocked(_) => ("cloudflare_blocked", 10),
                IherbError::ProductNotFound(_) => ("product_not_found", 11),
                IherbError::Navigation(_) => ("navigation_timeout", 12),
                IherbError::Json(_) => ("parse_failed", 13),
                _ => ("parse_failed", 13),
            };
        }
    }

    let msg = err.to_string().to_lowercase();
    if msg.contains("invalid") || msg.contains("cannot be empty") || msg.contains("at least 1") {
        ("invalid_input", 14)
    } else {
        ("parse_failed", 13)
    }
}

async fn cmd_search(
    config: &AppConfig,
    browser_session: &mut Option<BrowserSession>,
    query: &str,
    limit: usize,
    sort: SortOrder,
    category: Option<&str>,
    json_output: bool,
) -> Result<()> {
    let total_start = std::time::Instant::now();
    if query.trim().is_empty() {
        anyhow::bail!("Search query cannot be empty");
    }
    if limit == 0 {
        anyhow::bail!("Limit must be at least 1");
    }

    let cache = Cache::new(config.cache_dir.clone(), config.no_cache);

    if let Some(hit) = cache.get_search::<model::SearchResult>(query, sort, category) {
        let mut result = hit.data;
        result.products.truncate(limit);
        if json_output {
            println!("{}", output::format_search_json(&result)?);
        } else {
            print!("{}", output::format_search_results(&result));
            println!(
                "\n- **Data from:** {}",
                output::format_cached_at(hit.cached_at)
            );
        }
        return Ok(());
    }

    let session = get_or_launch_browser(config, browser_session).await?;
    let new_page_start = std::time::Instant::now();
    let page = session.new_page().await?;
    log_timing(config, "new_page_ms", new_page_start.elapsed(), None);
    let navigator = Navigator::new(config.delay_ms, config.timing);

    let base_url = config.base_url();
    let total_pages = scraper::search::pages_needed(limit);
    let mut all_products = Vec::new();
    let mut total_results = None;

    for page_num in 1..=total_pages {
        if all_products.len() >= limit {
            break;
        }

        let url = scraper::search::build_search_url(&base_url, query, sort, category, page_num);
        let html = navigator
            .navigate_with_retry(&page, &url, 2, scraper::navigation::ReadinessTarget::Search)
            .await
            .context("Failed to navigate to search page")?;

        let parse_start = std::time::Instant::now();
        let page_result =
            scraper::search::extract_search(&page, &html, query, &base_url, &config.currency)
                .await
                .context("Failed to extract search results")?;
        log_timing(
            config,
            &format!("search.page_{}.parse_results_ms", page_num),
            parse_start.elapsed(),
            Some(&format!("count={}", page_result.products.len())),
        );

        if page_result.products.is_empty() {
            break;
        }

        if total_results.is_none() {
            total_results = page_result.total_results;
        }

        all_products.extend(page_result.products);

        if page_num < total_pages {
            navigator.rate_limit_delay().await;
        }
    }

    if all_products.is_empty() {
        anyhow::bail!("No search results found for: {}", query);
    }

    // Cache the full result set before truncating
    let full_result = model::SearchResult {
        query: query.to_string(),
        total_results,
        products: all_products,
    };

    if let Err(e) = cache.set_search(query, sort, category, &full_result) {
        tracing::debug!("Failed to cache search results: {}", e);
    }

    let mut result = full_result;
    result.products.truncate(limit);

    if json_output {
        println!("{}", output::format_search_json(&result)?);
    } else {
        print!("{}", output::format_search_results(&result));
        println!(
            "\n- **Data from:** {}",
            output::format_cached_at(SystemTime::now())
        );
    }
    log_timing(
        config,
        "search.total_ms",
        total_start.elapsed(),
        Some(&format!(
            "count={} pages={}",
            result.products.len(),
            total_pages
        )),
    );
    Ok(())
}

async fn cmd_product(
    config: &AppConfig,
    browser_session: &mut Option<BrowserSession>,
    id_or_url: &str,
    section: Option<Section>,
    json_output: bool,
) -> Result<()> {
    let product_id = parse_product_identifier(id_or_url)?;
    let base_url = config.base_url();
    let url = product_url_for_input(id_or_url, &product_id, &base_url);
    let cache = Cache::new(config.cache_dir.clone(), config.no_cache);

    if let Some(hit) = cache.get_product::<model::ProductDetail>(&product_id) {
        let mut product = hit.data;
        product.product_url = url;
        if json_output {
            println!("{}", output::format_product_json(&product)?);
        } else {
            print!("{}", output::format_product_detail(&product, section));
            println!(
                "\n- **Data from:** {}",
                output::format_cached_at(hit.cached_at)
            );
        }
        return Ok(());
    }

    let total_start = std::time::Instant::now();
    let session = get_or_launch_browser(config, browser_session).await?;
    let new_page_start = std::time::Instant::now();
    let page = session.new_page().await?;
    log_timing(config, "new_page_ms", new_page_start.elapsed(), None);
    let navigator = Navigator::new(config.delay_ms, config.timing);

    let html = navigator
        .navigate_with_retry(
            &page,
            &url,
            2,
            scraper::navigation::ReadinessTarget::Product,
        )
        .await
        .context("Failed to navigate to product page")?;

    if scraper::helpers::is_not_found_page(&html) {
        anyhow::bail!("Product not found: {}", product_id);
    }

    let parse_start = std::time::Instant::now();
    let mut product =
        scraper::product::extract_product(&page, &html, &product_id, &base_url, &config.currency)
            .await
            .context("Failed to extract product data")?;
    log_timing(config, "product.parse_ms", parse_start.elapsed(), None);
    product.product_url = url.clone();

    // Validate the extracted product to catch nonexistent product pages that slip
    // through extraction (e.g., iHerb returns a page that doesn't trigger 404 detection
    // but has no real product data).
    if product.name.is_empty()
        || product.name == "Unknown Product"
        || (product.price == 0.0 && product.rating.is_none() && product.review_count.is_none())
    {
        anyhow::bail!("Product not found: {}", product_id);
    }

    if let Err(e) = cache.set_product(&product_id, &product) {
        tracing::debug!("Failed to cache product data: {}", e);
    }

    if json_output {
        println!("{}", output::format_product_json(&product)?);
    } else {
        print!("{}", output::format_product_detail(&product, section));
        println!(
            "\n- **Data from:** {}",
            output::format_cached_at(SystemTime::now())
        );
    }
    log_timing(config, "product.total_ms", total_start.elapsed(), None);
    Ok(())
}

async fn get_or_launch_browser<'a>(
    config: &AppConfig,
    session: &'a mut Option<BrowserSession>,
) -> Result<&'a BrowserSession> {
    if session.is_none() {
        let start = std::time::Instant::now();
        let chrome_path =
            browser::resolve::resolve_chrome(config.browser_path.as_ref(), &config.data_dir)
                .await
                .context("Failed to resolve Chrome browser")?;

        let launched = BrowserSession::launch(chrome_path, config)
            .await
            .context("Failed to launch browser")?;

        *session = Some(launched);
        log_timing(config, "browser_start_ms", start.elapsed(), None);
    }
    Ok(session.as_ref().unwrap())
}

fn log_timing(config: &AppConfig, phase: &str, elapsed: std::time::Duration, extra: Option<&str>) {
    if !config.timing {
        return;
    }
    if let Some(extra) = extra {
        eprintln!("[timing] {}={} {}", phase, elapsed.as_millis(), extra);
    } else {
        eprintln!("[timing] {}={}", phase, elapsed.as_millis());
    }
}

fn parse_product_identifier(input: &str) -> Result<String> {
    if input.chars().all(|c| c.is_ascii_digit()) && !input.is_empty() {
        return Ok(input.to_string());
    }

    if input.contains("iherb.com") {
        if let Some(id) = input
            .split('/')
            .rev()
            .find(|s| s.chars().all(|c| c.is_ascii_digit()) && !s.is_empty())
        {
            return Ok(id.to_string());
        }
    }

    anyhow::bail!(
        "Invalid product identifier: {}. Use a numeric ID or full iHerb URL",
        input
    );
}

fn product_url_for_input(input: &str, product_id: &str, base_url: &str) -> String {
    if input.contains("iherb.com") {
        input.to_string()
    } else {
        format!("{}/pr/item/{}", base_url, product_id)
    }
}
