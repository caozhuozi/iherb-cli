use crate::error::IherbError;
use chromiumoxide::Page;
use std::time::{Duration, Instant};

const MAX_CLOUDFLARE_RETRIES: u32 = 3;
const CLOUDFLARE_WAIT_SECS: u64 = 12;
const READINESS_TIMEOUT_MS: u64 = 8_000;
const READINESS_POLL_MS: u64 = 250;
const CLOUDFLARE_MARKERS: &[&str] = &[
    "Just a moment",
    "Attention Required",
    "请稍候",
    "正在进行安全验证",
    "Cloudflare",
    "cf-turnstile",
    "challenge-platform",
];

#[derive(Debug, Clone, Copy)]
pub enum ReadinessTarget {
    None,
    Product,
    Search,
}

pub struct Navigator {
    delay_ms: u64,
    timing: bool,
}

impl Navigator {
    pub fn new(delay_ms: u64, timing: bool) -> Self {
        Self { delay_ms, timing }
    }

    pub async fn navigate(
        &self,
        page: &Page,
        url: &str,
        readiness: ReadinessTarget,
    ) -> Result<String, IherbError> {
        tracing::info!("Navigating to: {}", url);

        let goto_start = Instant::now();
        page.goto(url)
            .await
            .map_err(|e| IherbError::Navigation(format!("Failed to navigate to {}: {}", url, e)))?;
        self.log_timing(
            &format!("{}.goto_ms", readiness.prefix()),
            goto_start,
            Some(url),
        );

        // Check for and handle Cloudflare challenge
        let cf_start = Instant::now();
        for attempt in 1..=MAX_CLOUDFLARE_RETRIES {
            if !self.is_cloudflare_challenge(page).await {
                break;
            }

            if attempt == MAX_CLOUDFLARE_RETRIES {
                return Err(IherbError::CloudflareBlocked(MAX_CLOUDFLARE_RETRIES));
            }

            tracing::info!(
                "Cloudflare challenge detected (attempt {}/{}), waiting up to {}s...",
                attempt,
                MAX_CLOUDFLARE_RETRIES,
                CLOUDFLARE_WAIT_SECS
            );

            // Try clicking the Cloudflare Turnstile checkbox (may fail due to cross-origin, but worth trying)
            let _ = page
                .evaluate(
                    r#"
                    try {
                        const iframe = document.querySelector('iframe[src*="challenges"]');
                        if (iframe && iframe.contentDocument) {
                            const checkbox = iframe.contentDocument.querySelector('input[type="checkbox"]');
                            if (checkbox) checkbox.click();
                        }
                    } catch(e) {}
                    "#,
                )
                .await;

            // Wait for Cloudflare to resolve, but check periodically for early exit
            let check_interval_ms = 1000;
            let total_checks = (CLOUDFLARE_WAIT_SECS * 1000) / check_interval_ms;
            for _ in 0..total_checks {
                tokio::time::sleep(Duration::from_millis(check_interval_ms)).await;
                if !self.is_cloudflare_challenge(page).await {
                    tracing::info!("Cloudflare challenge resolved early");
                    break;
                }
            }
        }
        self.log_timing(
            &format!("{}.cloudflare_check_ms", readiness.prefix()),
            cf_start,
            None,
        );

        self.wait_for_readiness(page, readiness).await?;

        if self.delay_ms > 0 {
            let delay_start = Instant::now();
            tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            self.log_timing(
                &format!("{}.configured_delay_ms", readiness.prefix()),
                delay_start,
                None,
            );
        }

        let html_start = Instant::now();
        let html = page
            .content()
            .await
            .map_err(|e| IherbError::Navigation(format!("Failed to get page content: {}", e)))?;
        self.log_timing(
            &format!("{}.html_extract_ms", readiness.prefix()),
            html_start,
            None,
        );

        Ok(html)
    }

    pub async fn navigate_with_retry(
        &self,
        page: &Page,
        url: &str,
        max_retries: u32,
        readiness: ReadinessTarget,
    ) -> Result<String, IherbError> {
        let mut last_err = None;

        for attempt in 1..=max_retries + 1 {
            match self.navigate(page, url, readiness).await {
                Ok(html) => return Ok(html),
                Err(e) => {
                    tracing::warn!(
                        "Navigation attempt {}/{} failed: {}",
                        attempt,
                        max_retries + 1,
                        e
                    );
                    last_err = Some(e);
                    if attempt <= max_retries {
                        let backoff = Duration::from_secs(2u64.pow(attempt - 1));
                        tracing::info!("Retrying in {:?}...", backoff);
                        tokio::time::sleep(backoff).await;
                    }
                }
            }
        }

        Err(last_err.unwrap())
    }

    async fn wait_for_readiness(
        &self,
        page: &Page,
        readiness: ReadinessTarget,
    ) -> Result<(), IherbError> {
        let selectors = readiness.selectors();
        if selectors.is_empty() {
            return Ok(());
        }

        let start = Instant::now();
        let selectors_json = serde_json::to_string(selectors).map_err(IherbError::Json)?;
        let script = format!(
            r#"
            (() => {{
                const selectors = {selectors_json};
                return selectors.some((selector) => document.querySelector(selector));
            }})()
            "#
        );

        while start.elapsed() < Duration::from_millis(READINESS_TIMEOUT_MS) {
            if self.is_cloudflare_challenge(page).await {
                return Err(IherbError::CloudflareBlocked(1));
            }

            let ready = page
                .evaluate(script.as_str())
                .await
                .ok()
                .and_then(|v| v.into_value::<bool>().ok())
                .unwrap_or(false);
            if ready {
                self.log_timing(
                    &format!("{}.wait_selector_ms", readiness.prefix()),
                    start,
                    None,
                );
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(READINESS_POLL_MS)).await;
        }

        self.log_timing(
            &format!("{}.wait_selector_ms", readiness.prefix()),
            start,
            Some("timeout=true"),
        );
        Ok(())
    }

    async fn is_cloudflare_challenge(&self, page: &Page) -> bool {
        match page.evaluate("[document.title, document.body ? document.body.innerText : '', document.documentElement ? document.documentElement.innerHTML : ''].join('\\n')").await {
            Ok(val) => {
                let content = val.into_value::<String>().unwrap_or_default();
                CLOUDFLARE_MARKERS
                    .iter()
                    .any(|marker| content.contains(marker))
            }
            Err(_) => false,
        }
    }

    pub async fn rate_limit_delay(&self) {
        tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
    }

    fn log_timing(&self, phase: &str, start: Instant, extra: Option<&str>) {
        if !self.timing {
            return;
        }
        if let Some(extra) = extra {
            eprintln!(
                "[timing] {}={} {}",
                phase,
                start.elapsed().as_millis(),
                extra
            );
        } else {
            eprintln!("[timing] {}={}", phase, start.elapsed().as_millis());
        }
    }
}

impl ReadinessTarget {
    fn prefix(self) -> &'static str {
        match self {
            ReadinessTarget::None => "navigation",
            ReadinessTarget::Product => "product",
            ReadinessTarget::Search => "search",
        }
    }

    fn selectors(self) -> &'static [&'static str] {
        match self {
            ReadinessTarget::None => &[],
            ReadinessTarget::Product => &[
                r#"script[type="application/ld+json"]"#,
                "h1#name",
                r#"h1[data-testid="product-name"]"#,
                "#product-specs-list",
                ".product-image-gallery",
                "#product-overview",
            ],
            ReadinessTarget::Search => &[
                "div.product-cell-container",
                "a.absolute-link.product-link",
                "a.product-link",
                "#product-count",
                ".no-results",
            ],
        }
    }
}
