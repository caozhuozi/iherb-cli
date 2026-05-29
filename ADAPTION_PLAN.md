# iHerb CLI Scraper Adaptation Plan

## Background

The current `iherb-cli` is a Rust CLI that can search iHerb and fetch product details through a Chromium browser. It is useful as a single-query tool, but it is not yet ideal as a low-level fetcher for a separate bulk scraper.

The intended architecture is:

```text
bulk scraper
-> starts with iherb-cli search --json to discover product URLs
-> writes discovered product URLs into a durable queue
-> calls iherb-cli product --json for one queued URL at a time
-> stores structured product JSON
-> handles retries, checkpointing, failed queue, rate limits, and Computer Use recovery
```

The CLI should stay focused on fetching one search page or one product detail. The scraper should own bulk crawling.

The scraper should target the US storefront and English-language pages only:

```text
https://www.iherb.com
language: English
```

Do not crawl or normalize multiple regional storefronts or non-English pages in the first version. Product URLs should be discovered and fetched from the US site with English page content.

## Observed Behavior

Tested commands:

```bash
cargo run -- search "vitamin c" --limit 1
```

This succeeded and returned a product with a full URL. The scraper-facing examples below use this English product page as the reference shape:

```text
https://www.iherb.com/pr/california-gold-nutrition-gold-c-powder-usp-grade-vitamin-c-1-000-mg-8-81-oz-250-g/59561
```

Running product details initially failed because the product page returned a Cloudflare challenge page:

```text
<title>请稍候…</title>
正在进行安全验证
Cloudflare
```

After logging in through a persisted Chrome profile, product fetching eventually succeeded:

```bash
cargo run -- product "https://www.iherb.com/pr/california-gold-nutrition-gold-c-powder-usp-grade-vitamin-c-1-000-mg-8-81-oz-250-g/59561"
```

Example extracted Product Code shape:

```text
Product Code: <product code from page>
```

The future scraper-facing command should not require a section argument. It should return the full target JSON in one call:

```bash
cargo run -- product "https://www.iherb.com/pr/california-gold-nutrition-gold-c-powder-usp-grade-vitamin-c-1-000-mg-8-81-oz-250-g/59561" --json --delay 5000
```

Example output:

```text
Supplement Facts:
Vitamin C (as Ascorbic Acid) | 1,000 mg | 1,111%

Other Ingredients:
None.
```

## Important Direction

Do not turn `iherb-cli` into a full-site crawler.

Instead, make it a scraper-friendly fetcher:

```text
input: search query or full product URL
output: structured JSON
errors: structured JSON + stable exit codes
browser state: controlled by explicit profile dir
```

The bulk scraper can then call it repeatedly and handle scheduling.

## URL Handling

For scraper use, full product URLs should be preferred and preserved exactly.

Recommended behavior:

```text
if input is a full iHerb URL:
navigate to that exact URL

if input is only numeric product ID:
keep existing fallback behavior, or clearly document that full URL is preferred
```

Avoid making unrelated route changes such as `/pr/item/{id}` to `/pr/p/{id}` unless verified necessary. For a scraper, the safest path is to use the exact URL obtained from search results.

Suggested helper:

```rust
fn product_url_for_input(input: &str, product_id: &str, base_url: &str) -> String {
if input.contains("iherb.com") {
input.to_string()
} else {
format!("{}/pr/item/{}", base_url, product_id)
}
}
```

If `/pr/item/{id}` is known broken, consider requiring full URLs for product fetches instead of guessing short routes.

## Chrome Profile Strategy

Cloudflare is much easier to handle if browser state persists.

Current state:

```text
--debug is already supported.
--profile-dir is not supported yet and must be added as part of the scraper adaptation.
```

The combination of `--debug` and `--profile-dir` is critical for first-time session setup and later recovery:

```text
--debug opens a visible browser so the user can log in, choose US/English, and complete Cloudflare manually.
--profile-dir tells the CLI where to save/reuse that browser profile.
```

The current temporary profile approach is clean but bad for Cloudflare:

```text
each run looks like a fresh browser
login state is lost
Cloudflare clearance cookies are lost
site/session preferences are lost
```

For scraper use, add explicit profile control:

```bash
iherb-cli product "$url" --json --profile-dir ./runtime/chrome-profile
```

Recommended rules:

```text
default behavior can remain temporary profile
--profile-dir enables persistent browser state
profile lifecycle is owned by the caller/scraper
CLI must not delete a caller-provided profile
```

This allows:

```text
manual login or Computer Use recovery
-> writes cookies to ./runtime/chrome-profile
-> later CLI calls reuse the same browser state
```

The recovery/login browser session must be set to the US storefront and English language. Avoid saving Chinese or other non-English language preferences into the scraper profile, because the parser and target database are designed for US English source text.

Important: the user must configure language/region in the same Chrome profile that `iherb-cli` uses. Setting iHerb to US/English in the user's normal daily Chrome profile does not help if the CLI uses a separate `--profile-dir`.

Recommended first-time setup:

```bash
iherb-cli --debug --profile-dir ./runtime/chrome-profile
```

Equivalent explicit setup command:

```bash
iherb-cli --debug --profile-dir ./runtime/chrome-profile setup
```

In the opened browser:

```text
1. Complete any Cloudflare verification manually.
2. Log in to iHerb if needed.
3. Set country/region to United States.
4. Set language to English.
5. Set currency to USD if the site asks.
6. Confirm the host is www.iherb.com and the page text is English.
7. Press Ctrl+C in the terminal when setup is done.
```

After this, the scraper and CLI should reuse the same `./runtime/chrome-profile` so cookies, Cloudflare clearance, login state, and US/English preferences persist.

## Cloudflare Handling

The CLI should detect Cloudflare and return a structured error. It should not misreport Cloudflare as `Product not found`.

Detected challenge markers should include:

```text
Just a moment
Attention Required
请稍候
正在进行安全验证
Cloudflare
cf-turnstile
challenge-platform
```

Recommended behavior:

```json
{
"ok": false,
"error_type": "cloudflare_blocked",
"url": "https://...",
"message": "Cloudflare challenge detected"
}
```

Recommended exit codes:

```text
0 success
10 cloudflare_blocked
11 product_not_found
12 navigation_timeout
13 parse_failed
14 invalid_input
```

Retry policy inside CLI should be bounded. The bulk scraper should decide whether to back off, retry later, or trigger Computer Use recovery.

## Computer Use Integration Plan

Computer Use should live in the external scraper/recovery layer, not inside the core `iherb-cli` product fetch path.

Recommended flow:

```text
scraper calls:
iherb-cli product "$url" --json --profile-dir ./runtime/chrome-profile

if success:
write JSONL

if cloudflare_blocked:
scraper starts Computer Use recovery using the same profile dir
Computer Use/人工操作 opens headed Chrome
wait for Cloudflare/login session to recover
close browser
scraper retries iherb-cli product with same profile dir
```

Computer Use should not be used to fully automate CAPTCHA solving, password entry, 2FA, account security pages, payment pages, or order/account data access.

Safe role for Computer Use:

```text
detect current page state
wait for simple challenge completion
complete ordinary interactive Cloudflare checks, including click-and-hold challenges
click ordinary non-sensitive buttons
confirm product page has loaded
ask human to intervene when login/CAPTCHA/account security appears
```

iHerb/Cloudflare may show an interactive click-and-hold human verification challenge. The recovery layer should let Computer Use complete this challenge in a headed browser using the same `--profile-dir`, then wait until the browser returns to the target iHerb page before retrying the CLI fetch. If the flow changes into a CAPTCHA, login, password, 2FA, account security, payment, or order/account-data page, Computer Use should stop and ask for human intervention instead of continuing.

## Delay and Rate Limiting

`--delay` does not solve Cloudflare. It only reduces risk and gives scripts time to complete.

Useful effects:

```text
slower request cadence looks less automated
Cloudflare JS gets more time to set cookies and redirect
fewer repeated timeouts/retries
```

For bulk product details:

```bash
--delay 10000
```

is safer than the default.

Rough estimate from one test:

```text
supplement search returned about 28,529+ results
28,529 products * 10 seconds = about 79.2 hours = about 3.3 days
realistic total with page load/retries/Cloudflare = 4-7+ days
```

The scraper must support checkpointing and resume.

## JSON Output

Markdown is not suitable for a bulk scraper. Add:

```bash
--json
```

or:

```bash
--format markdown|json
```

Preferred product command:

```bash
iherb-cli product "$url" --json --profile-dir ./runtime/chrome-profile --delay 10000
```

Preferred search command:

```bash
iherb-cli search "supplement" --category supplements --limit 48 --json --delay 10000
```

For scraper use, `search --json` should only output stable discovery fields. Do not include dynamic commerce fields such as price, original price, currency, rating, review count, or stock status in the scraper-facing JSON.

## Product Data Needed

The scraper needs more than current overview/ingredients output.

The example below uses the English database shape for product `59561`. Direct `curl` of the page returned a Cloudflare `Just a moment...` challenge in this environment, so page-derived values should still be verified by the final scraper against live product HTML.

For the scraper-facing CLI, remove the need for `--section`. `product --json` should always return the complete product JSON needed by the database in one call. The old sectioned Markdown output can be kept only as a human-facing compatibility mode or deprecated later.

Target product JSON should include:

```json
{
"ok": true,
"data": {
"product_id": "59561",
"product_code": "<product code from page>",
"product_url": "https://...",
"name": "California Gold Nutrition, Gold C Powder, USP Grade Vitamin C, 1,000 mg, 8.81 oz (250 g)",
"brand": "California Gold Nutri·tion",
"image_url": "https://...",
"image_urls": ["https://..."],
"category_breadcrumb": ["..."],
"certifications_and_diet": [
"Vegetarian",
"Gluten-free",
"Soy-free"
],
"suggested_use": "Mix 1 scoop daily with water or the beverage of your choice. Best when taken as directed by a qualified healthcare professional.\n\nNote: 1 g (1,000 mg) per scoop is an average. Individual scooping technique may yield slightly less than or slightly more than 1 g.",
"other_ingredients": "Main Ingredients\nVitamin C (as Ascorbic Acid)\n\nOther Ingredients\nNone\n\nNot manufactured with milk, eggs, fish, crustacean shellfish, tree nuts, peanuts, wheat, soy, sesame, or gluten. Produced in an FDA-registered, third-party audited, and cGMP-compliant facility that may process other products that contain these allergens or ingredients.",
"supplement_facts": {
"serving_size": "1 scoop",
"servings_per_container": "250",
"nutrients": [
{
"name": "Vitamin C (as Ascorbic Acid)",
"amount": "1,000 mg",
"daily_value": "1,111%"
}
]
},
"warnings": "..."
}
}
```

## Image URLs

Add product image fields:

```rust
pub image_url: Option<String>,
pub image_urls: Vec<String>,
```

Extraction priority:

```text
JSON-LD Product.image
Open Graph meta property="og:image"
DOM fallback for product image elements
```

Possible selectors:

```text
meta[property="og:image"]
img[itemprop="image"]
img#iherb-product-image
.product-image img
```

Normalize relative URLs to absolute URLs when needed.

## Product Detail Extraction

The product scraper should extract all required product fields in one pass. Do not require scraper callers to choose `--section overview`, `--section ingredients`, `--section nutrition`, or similar section-specific modes.

Needed source sections/areas:

```text
Important Information
Suggested Use
Other Ingredients
Supplement Facts
Warnings
Product specs
Images
Breadcrumb/category
```

The first scraper version should fetch US English pages only. The database should store normalized English field names and English source text.

```text
certifications_and_diet
suggested_use
other_ingredients
```

Scraper-facing product JSON must not include `key_info` or `country_of_origin`. Certifications and diet labels should be exposed directly as a top-level `certifications_and_diet` array. If no stable structured labels are found, output an empty array instead of `null`.

The extraction code should prioritize English headings from the US storefront. Localized heading support can be added later if needed.

### Other Ingredients for Complex Supplements

The iHerb product page heading is usually `Other ingredients`, but the actual DOM block can contain more than only inactive ingredients. This is especially important for complex supplements such as B-complex and multivitamin products.

Observed fixture shapes:

```text
single nutrient product:
Main Ingredients
Vitamin C (as Ascorbic Acid)

Other Ingredients
None

allergen/manufacturing statement
```

```text
complex supplement product:
Main Ingredients
long active ingredient list

Other Ingredients
capsule/excipient list

allergen/manufacturing statement

optional formula, trademark, or branded-ingredient notes
```

Recommended scraper-facing behavior:

```text
store the whole `Other ingredients` section as raw normalized section text
do not split it into unstable subfields in the first version
preserve paragraph boundaries where possible
```

Reason:

```text
for complex formulas, the page's `Other ingredients` section includes main active ingredients, excipients, allergen/manufacturing statements, and sometimes trademark notes. Splitting this block too early risks losing or misclassifying source text.
```

`product --json` should expose this as:

```json
{
"other_ingredients": "Main Ingredients\n...\n\nOther Ingredients\n...\n\nNot manufactured with ...\n\nFormulated with ..."
}
```

As with `search --json`, scraper-facing `product --json` should not expose dynamic commerce fields such as price, original price, currency, rating, review count, or stock status. These can remain available internally for human-facing Markdown compatibility, but they are not part of the product information database target JSON.

## Bulk Scraper Responsibilities

The external scraper should handle:

```text
seed discovery from search
URL deduplication
calling iherb-cli product --json
writing products.jsonl immediately after each success
writing failed.jsonl for failures
checkpoint/resume
rate limiting
backoff
Cloudflare recovery trigger
manual/Computer Use recovery
final failed retry pass
```

## Scraper Start Step and SQLite Queue

The scraper should start with a discovery step. It should not begin by manually hardcoding product URLs.

Discovery command target:

```bash
iherb-cli search "supplement" --category supplements --json --limit 1000 --profile-dir ./runtime/chrome-profile --delay 10000
```

The `search --json` output should include at least:

```json
{
"ok": true,
"data": {
"query": "supplement",
"total_results": 28529,
"products": [
{
"product_id": "59561",
"product_code": "CGN-00935",
"product_url": "https://www.iherb.com/pr/california-gold-nutrition-gold-c-powder-usp-grade-vitamin-c-1-000-mg-8-81-oz-250-g/59561",
"name": "California Gold Nutrition, Gold C Powder, USP Grade Vitamin C, 1,000 mg, 8.81 oz (250 g)",
"brand": "California Gold Nutrition"
}
]
}
}
```

The scraper should upsert these discovered products into a SQLite-backed queue.

The saved `iherb-Search-vitamin.html` fixture shows that main search result cards expose both the slug-style product URL and the iHerb Product Code directly on the product link:

```html
<a
class="absolute-link product-link"
href="https://www.iherb.com/pr/.../79975"
data-product-id="79975"
data-part-number="SRE-01134"
data-ga-brand-name="Sports Research"
title="Sports Research, D3 + K2, Plant Based, 125 mcg/100 mcg, 60 Veggie Softgels">
</a>
```

Some cards also include a hidden `sku` field that contains the same iHerb Product Code:

```html
<div itemprop="sku" content="SRE-01134"></div>
```

Therefore `search --json` should extract `product_code` from `data-part-number` first, then fall back to hidden `itemprop="sku"` when available. It should store the full slug-style `product_url`, but it does not need to store the slug as a separate field.

Scraper-facing `search --json` should not expose price, original price, currency, rating, review count, or stock status. Those fields are dynamic and are not needed for the product information database.

Recommended SQLite schema:

```sql
CREATE TABLE IF NOT EXISTS product_queue (
product_id TEXT PRIMARY KEY,
url TEXT NOT NULL,
status TEXT NOT NULL,
attempts INTEGER NOT NULL DEFAULT 0,
last_error_type TEXT,
last_error_message TEXT,
created_at TEXT NOT NULL,
updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS product_data (
product_id TEXT PRIMARY KEY,
url TEXT NOT NULL,
data_json TEXT NOT NULL,
scraped_at TEXT NOT NULL
);
```

Recommended queue statuses:

```text
pending
processing
done
failed
cloudflare_blocked
```

Recommended crawl loop:

```text
1. SELECT one pending product from product_queue.
2. Mark it processing and increment attempts.
3. Run:
iherb-cli product "$url" --json --profile-dir ./runtime/chrome-profile --delay 10000
4. On success:
- upsert product_data
- set queue status to done
5. On cloudflare_blocked:
- set queue status to cloudflare_blocked
- pause crawl
- recover the same profile manually or with allowed Computer Use assistance
- set blocked rows back to pending and retry
6. On parse/navigation/product errors:
- if attempts < max attempts, set status back to pending
- otherwise set status to failed and keep last_error_type/message
```

This queue is how the scraper knows what to crawl, what has already been crawled, what failed, and what needs retry after Cloudflare/session recovery.

Suggested failed record:

```json
{
"url": "https://...",
"product_id": "59561",
"error_type": "cloudflare_blocked",
"message": "...",
"attempts": 3,
"last_seen_at": "2026-05-25T..."
}
```

Suggested success record format:

```text
one product JSON object per line in products.jsonl
```

## Recommended Minimal Implementation Order

1. Add `--json` output for `product` and `search`.
2. Add structured JSON errors and stable exit codes.
3. Add `--profile-dir` and make profile cleanup conditional.
4. Preserve full product URL exactly.
5. Improve Cloudflare detection so it returns `cloudflare_blocked`.
6. Add image URL extraction.
7. Add important information extraction.
8. Extract `suggested_use` and `other_ingredients` as section-level raw text, without splitting their internal prose into unstable subfields.
9. Add tests for parsers using saved HTML fixtures.
10. Build the external bulk scraper separately.

## Do Not Prioritize

These are not important for the scraper adaptation:

```text
changing /pr/item to /pr/p without strong evidence
making CLI crawl all products itself
parsing Markdown in the external scraper
fully automating CAPTCHA/login/password/2FA
unbounded retry loops
parallelizing heavily before Cloudflare behavior is stable
```

## Scraper Readiness, Breadcrumbs, and Image URL Notes

This section captures implementation notes for future Codex sessions modifying `iherb-cli` into a scraper-facing CLI used by a separate bulk scraper.

### Core Principle: Scrape Text and URLs First

The first scraper version is a metadata scraper, not an image downloader.

Primary targets:

```text
product text
product identifiers
product URLs
image URLs
category/breadcrumb text
ingredient and usage text
supplement facts text/table data
```

Optimize waiting around the moment when required text and URL fields are available. Do not optimize around full browser page completion, image binary loading, review widgets, recommendations, analytics scripts, or other nonessential assets.

In practical terms:

```text
if product title, product code, required text sections, and image URL strings are present, the page is ready enough
do not wait for image files to finish loading
do not wait for recommendation images or review widgets
do not download images in this phase
leave image binary download and dimension verification to a later pipeline
```

This principle applies to both `product` and `search` command behavior.

### Breadcrumb / Category Path

The scraper should record the product category breadcrumb when it is available on product pages.

Example visible breadcrumb:

```text
Categories > Supplements > Vitamins > Vitamin B > Vitamin B12 (Cobalamin) > Methylcobalamin
```

Expected stored JSON:

```json
{
  "category_breadcrumb": [
    "Supplements",
    "Vitamins",
    "Vitamin B",
    "Vitamin B12 (Cobalamin)",
    "Methylcobalamin"
  ]
}
```

Do not store the UI label `Categories` unless the page uses it as a real category node. In this example, `Categories` is only a label before the actual path.

Current implementation note:

```text
ProductDetail already has a category_breadcrumb field.
The current parser still fills it with None.
Breadcrumb extraction still needs to be implemented.
```

Because this is text metadata, it belongs in the first scraper version. It should be extracted from the product page HTML/DOM and should not require image loading or full-page `document.readyState == "complete"`.

### Avoid Blind ReadyState Complete Waits

Blindly waiting for `document.readyState == "complete"` can make iHerb scraping much slower than necessary. On modern ecommerce pages, the data needed by the scraper may appear before the whole page is complete.

The page can continue loading nonessential resources after useful product content is already present:

```text
recommendation carousels
review widgets
tracking scripts
advertising scripts
personalization scripts
deferred images
regional/language preference scripts
Cloudflare or bot-detection related scripts
```

Waiting for full completion means the CLI may waste time waiting for resources irrelevant to scraper output. This can make runtime much longer than the configured `--delay`. For example, even if `--delay 10000` means "wait 10 seconds after navigation", `page.goto(url)` or the readiness check may already have spent much more time before that delay starts.

Browser navigation wait modes have different costs:

```text
load: traditional full page load event
domcontentloaded: HTML document parsed, but images/scripts may still be loading
networkidle: no or very few network requests for a period
custom selector wait: wait only until the specific DOM elements needed by the scraper exist
```

For scraper usage, waiting for `load`, `networkidle`, or `document.readyState == "complete"` is usually too conservative.

Recommended readiness model:

```text
1. Navigate with a lighter wait target, such as DOMContentLoaded.
2. Wait for one or more product-specific selectors.
3. Extract data as soon as required content is present.
4. Use a short bounded fallback wait if dynamic sections are not ready.
5. Return structured errors if the product page, Cloudflare page, or locale redirect cannot be resolved.
```

Product readiness signals:

```text
JSON-LD product script exists
product title exists
product ID / SKU / product code exists
product image gallery exists
important sections exist when available: Suggested Use, Other Ingredients, Supplement Facts, Key Info
```

The scraper does not need to wait for every image, recommendation, review, or tracking request.

### Scraper-Facing CLI Behavior

For scraper mode:

```bash
iherb-cli product <url> --json --profile-dir ./runtime/chrome-profile
```

The command should:

```text
preserve the input product URL when it is already a full iHerb URL
target US English pages only
navigate with a bounded timeout
wait for product-specific selectors, not full readyState complete
detect Cloudflare challenge pages early
emit timing logs for each navigation and parsing phase
return structured JSON on success
return structured errors on failure
```

Example structured error:

```json
{
  "ok": false,
  "error_type": "cloudflare_blocked",
  "message": "Cloudflare challenge detected before product content loaded",
  "url": "https://www.iherb.com/pr/example-product/12345"
}
```

Do not treat Cloudflare pages as product pages. Detect challenge pages using text or DOM markers such as:

```text
Just a moment
Attention Required
Cloudflare
cf-turnstile
challenge-platform
请稍候
正在进行安全验证
```

When detected, return a structured `cloudflare_blocked` result so the outer scraper can pause, retry later, or ask for manual recovery.

### Timing Logs

Add explicit timing logs around the browser navigation pipeline. Slow runs may be caused by different phases:

```text
browser startup
profile loading
page.goto(url)
waiting for DOMContentLoaded
waiting for product selectors
waiting for Cloudflare/manual recovery
intentional --delay
HTML extraction
DOM parsing
image URL candidate extraction
JSON serialization
```

Without phase-level timing logs, it is easy to misread the problem and blame `--delay` or `document.readyState == "complete"` when the actual cost is browser launch, profile startup, network latency, or a blocked challenge page.

Plain stderr log shape:

```text
[timing] browser_start_ms=842
[timing] new_page_ms=31
[timing] goto_ms=12874 url=https://www.iherb.com/pr/example/12345
[timing] wait_domcontentloaded_ms=215
[timing] wait_product_selector_ms=642
[timing] cloudflare_check_ms=18 blocked=false
[timing] configured_delay_ms=10000
[timing] html_extract_ms=96
[timing] parse_product_ms=44
[timing] image_candidates_ms=12 count=9
[timing] total_ms=23814
```

For scraper JSON mode, send timing logs to stderr, not stdout. Stdout must remain valid machine-readable JSON.

If the CLI later supports structured logging, use JSON lines on stderr:

```json
{"level":"debug","event":"timing","phase":"goto","duration_ms":12874,"url":"https://www.iherb.com/pr/example/12345"}
{"level":"debug","event":"timing","phase":"wait_product_selector","duration_ms":642}
{"level":"debug","event":"timing","phase":"total","duration_ms":23814}
```

Recommended CLI flag:

```bash
--timing
```

The future scraper can enable it during debugging:

```bash
iherb-cli product <url> --json --timing --profile-dir ./runtime/chrome-profile
```

### Product and Search Need Different Readiness Strategies

Do not use one generic "wait until complete, then parse everything" strategy for both product pages and search pages.

Product pages and search result pages have different scraper goals, readiness signals, and failure modes.

Product page goal:

```text
extract one complete product record
```

Product readiness should focus on product-specific content:

```text
product JSON-LD exists
product title exists
product ID / product code exists
image gallery exists or product image fallback exists
target content sections exist when available: Suggested Use, Other Ingredients, Supplement Facts, Key Info
```

Product output should include:

```text
product_id
product_code
product_url
name
brand
image_url
image_urls
category_breadcrumb
certifications_and_diet
suggested_use
other_ingredients
supplement_facts
warnings
```

Product pages should spend more effort on extraction quality because each page represents one final product record. For product image extraction, record image URLs only. Use the product gallery and high-resolution Cloudinary URL variants such as `/y/`, but do not download image files during the product scrape.

Search page goal:

```text
discover product URLs and IDs for the outer scraper queue
```

Search readiness should focus only on search result cards/links:

```text
result container exists
product result links exist
pagination / next page state can be detected
no-results state can be detected
Cloudflare challenge can be detected
```

Search should not wait for product detail content, reviews, image gallery, supplement facts, or ingredient sections. Those belong to product pages. Search should also not download images. If a search result exposes a useful image URL cheaply in the HTML, it may record that URL, but binary image downloading belongs to a later separate pipeline.

Search page output should stay small and stable:

```json
{
  "items": [
    {
      "product_id": "79975",
      "product_code": "SRE-01134",
      "product_url": "https://www.iherb.com/pr/sports-research-d3-k2-plant-based-125-mcg-100-mcg-60-veggie-softgels/79975",
      "name": "Sports Research, D3 + K2, Plant-Based, 125 mcg / 100 mcg, 60 Veggie Softgels",
      "brand": "Sports Research"
    }
  ]
}
```

Do not include scraper-unneeded volatile fields in search JSON:

```text
price
original price
currency
rating
review count
stock status
```

Search result pages can extract product identifiers from result links and attributes such as:

```text
href
data-product-id
data-part-number
data-ga-brand-name
title
fallback hidden itemprop="sku"
```

Search pagination should be controlled by `--limit`, but `--limit` is not necessarily a URL parameter. The current behavior fetches enough result pages to satisfy the limit and then truncates. The existing code assumes 48 results per page, so `--limit 100` means fetch 3 search pages, then return the first 100 discovered items.

Use different timing phase names for product and search:

```text
[timing] product.goto_ms=12874
[timing] product.wait_jsonld_ms=215
[timing] product.wait_gallery_ms=642
[timing] product.parse_ms=44
[timing] product.total_ms=23814

[timing] search.page_1.goto_ms=8362
[timing] search.page_1.wait_results_ms=411
[timing] search.page_1.parse_results_ms=37 count=48
[timing] search.page_2.goto_ms=7920
[timing] search.total_ms=28410 count=100 pages=3
```

### Product Image Extraction

iHerb product images are served from Cloudinary with predictable path segments.

Cloudinary product image path pattern:

```text
https://cloudinary.images-iherb.com/image/upload/f_auto,q_auto:eco/images/{brand_code}/{product_code_lower_no_dash}/{size}/{image_id}.jpg
```

Example:

```text
https://cloudinary.images-iherb.com/image/upload/f_auto,q_auto:eco/images/cgn/cgn00931/y/388.jpg
```

The size is controlled by the path segment before the image filename:

```text
/s/
/r/
/g/
/l/
/y/
```

Observed practical priority:

```text
/y/ > /l/ > /g/ > /r/ > /s/
```

Important: `/g/` is not necessarily the maximum size.

In the analyzed saved page for California Gold Nutrition, Gold C, USP Grade Vitamin C, 1,000 mg, 60 Veggie Capsules:

```text
product_id: 61864
product_code: CGN-00931
JSON-LD uses /g/383.jpg
Open Graph metadata uses /s/
thumbnails use /s/ and /r/
large gallery references use /l/
the actual largest saved product image is /y/388.jpg at 1600x1600
```

Largest URL found in the HTML:

```text
https://cloudinary.images-iherb.com/image/upload/f_auto,q_auto:eco/images/cgn/cgn00931/y/388.jpg
```

Relevant HTML shape:

```html
<img class="lazy img-responsive"
  data-lazyload="https://cloudinary.images-iherb.com/image/upload/f_auto,q_auto:eco/images/cgn/cgn00931/y/388.jpg"
  alt=""
  data-image-type="Main"
  data-image-index="388">
```

Thumbnail/gallery shape:

```html
<img
  srcset="https://cloudinary.images-iherb.com/image/upload/f_auto,q_auto:eco/images/cgn/cgn00931/s/388.jpg 1x,
https://cloudinary.images-iherb.com/image/upload/f_auto,q_auto:eco/images/cgn/cgn00931/r/388.jpg 1.5x"
  data-large-img="https://cloudinary.images-iherb.com/image/upload/f_auto,q_auto:eco/images/cgn/cgn00931/l/388.jpg">
```

Do not rely only on:

```text
og:image
JSON-LD image
the first visible img src
search result thumbnail URLs
```

Those often point to smaller variants. JSON-LD and Open Graph are useful fallbacks, but not reliable sources for the largest image.

Recommended image logic:

```text
collect image URLs from product gallery data-lazyload
collect image URLs from product gallery data-large-img
collect image URLs from srcset
collect image URLs from JSON-LD image
collect image URLs from Open Graph og:image
filter for iHerb Cloudinary product image URLs
extract product image IDs, such as 383.jpg and 388.jpg
group all discovered URLs by image ID
select the best known variant using /y/ > /l/ > /g/ > /r/ > /s/
keep one URL string per image ID
choose the best high-resolution candidate as image_url
store selected URL strings in image_urls
```

For the first scraper, "best" means "best variant already visible in the product page HTML". The metadata scraper should not perform HTTP existence checks, image downloads, or dimension checks.

The later image downloader pipeline can derive all variant candidates from any stored URL:

```text
https://cloudinary.images-iherb.com/image/upload/f_auto,q_auto:eco/images/cgn/cgn00931/s/388.jpg
https://cloudinary.images-iherb.com/image/upload/f_auto,q_auto:eco/images/cgn/cgn00931/r/388.jpg
https://cloudinary.images-iherb.com/image/upload/f_auto,q_auto:eco/images/cgn/cgn00931/g/388.jpg
https://cloudinary.images-iherb.com/image/upload/f_auto,q_auto:eco/images/cgn/cgn00931/l/388.jpg
https://cloudinary.images-iherb.com/image/upload/f_auto,q_auto:eco/images/cgn/cgn00931/y/388.jpg
```

Because the variant can be derived from the URL path, the metadata scraper does not need to store a separate `variant` field.

Product JSON image fields should stay simple:

```json
{
  "image_url": "https://cloudinary.images-iherb.com/image/upload/f_auto,q_auto:eco/images/cgn/cgn00931/y/388.jpg",
  "image_urls": [
    "https://cloudinary.images-iherb.com/image/upload/f_auto,q_auto:eco/images/cgn/cgn00931/y/388.jpg",
    "https://cloudinary.images-iherb.com/image/upload/f_auto,q_auto:eco/images/cgn/cgn00931/y/383.jpg"
  ]
}
```

`image_url` should be the best primary image. `image_urls` should contain useful product gallery images, preferably high-resolution variants.

The metadata scraper should not store:

```text
variant
width
height
file_size
content_type
local_path
image download status
```

Those belong to the later image pipeline.

Images can usually be downloaded directly later with a normal HTTP client such as `wget`, `curl`, or the scraper's own downloader. Keep product metadata scraping and binary asset downloading separate:

```text
store image URLs in product JSON first
download images in a separate pipeline step
retry image downloads independently from product page scraping
verify HTTP status and dimensions in the image pipeline
avoid blocking the whole product scraper if one image download fails
```

The first scraper is responsible for:

```text
discovering product URLs
extracting product metadata
extracting image URLs
storing image URLs with the product record
```

It is not responsible for:

```text
downloading image files
checking final image dimensions
parsing image headers
retrying failed image binary downloads
deciding local image storage paths
```

Parsing image `width` and `height` is cheap in CPU terms because JPEG/PNG/WebP dimensions are available from headers or early bytes. The expensive part is network I/O: HTTP requests, downloading bytes, retries, and rate limiting. That is why dimensions should still be handled by the later image pipeline, not by the metadata scraper.

Do not overfit image extraction to one product. Implement a generic extractor that:

```text
scans all relevant image attributes
filters for iHerb Cloudinary product image URLs
groups by product code path when possible
prefers /y/ variants as URL candidates
outputs stable JSON for the outer scraper
```

### RapidAPI Product Data Assessment

There is a paid/third-party data option on RapidAPI:

```text
https://rapidapi.com/daniel.hpassos/api/iherb-product-data-api
```

The page identifies the API as:

```text
IHerb Product Data Api
```

The user-provided example response looks more like an ecommerce product feed than a clean supplement ingredient database.

Fields that overlap with the target schema:

```text
productId -> product_id
sku -> product_code
link -> product_url
title -> name
brandName -> brand
productCatalogImage -> image_url
productImages -> image_urls
categories -> category_breadcrumb, but may be incomplete
supplementFacts.servingSize -> supplement_facts.serving_size
supplementFacts.servingsPerContainer -> supplement_facts.servings_per_container
supplementFacts.nutritionalFacts -> supplement_facts.nutrients
nutritionalFacts.substancy -> nutrient.name
nutritionalFacts.amountPerServing -> nutrient.amount
nutritionalFacts.dailyValuePercent -> nutrient.daily_value
```

Fields intentionally out of scope for the first scraper:

```text
price
formattedPrice
formattedSpecialPrice
specialPrice
formattedTrialPrice
trialPrice
discountPercentValue
hasDiscount
soldPercent
ratingValue
reviewCount
outOfStock
stockLeft
shippingWeight
dimensions
bestByApproximately
lastUpdate
currencyUsed
countryUsed
languageUsed
unitsOfMeasureUsed
```

These are ecommerce/marketplace fields. The current project goal is supplement text metadata, ingredient data, category path, and image URLs.

The RapidAPI example does not clearly provide separate structured fields for:

```text
suggested_use
other_ingredients
warnings
certifications_and_diet
full category breadcrumb
```

It has `allDescription`, which may contain some of this text, but that is not the same as stable structured fields. `allDescription` may be a mixed blob containing product description, marketing claims, suggested use, other ingredients, warnings, disclaimers, or only some of them.

Do not assume `allDescription` is equivalent to:

```json
{
  "suggested_use": "...",
  "other_ingredients": "...",
  "warnings": "..."
}
```

Before using this paid API as a replacement for scraping, obtain real responses for several supplement products and verify:

```text
1. Does allDescription consistently include Suggested Use?
2. Does allDescription consistently include Other Ingredients?
3. Does allDescription consistently include Warnings?
4. Are section headings preserved?
5. Can these sections be split reliably?
6. Are categories a full breadcrumb or only coarse category tags?
7. Are country of origin and certifications/diet available anywhere?
```

Assessment:

```text
possible supplemental data source
possible shortcut for product discovery or supplement facts
not a confirmed replacement for this scraper
```

The API may cover product catalog fields and `supplement_facts.nutrients`, but it does not clearly cover the structured ingredient/use/warning sections that matter for this project.

## Useful Commands

Search sample:

```bash
cargo run -- search "vitamin c" --limit 1 --delay 5000
```

Current human-facing product command:

```bash
cargo run -- product "https://www.iherb.com/pr/california-gold-nutrition-gold-c-powder-usp-grade-vitamin-c-1-000-mg-8-81-oz-250-g/59561" --delay 5000
```

Future scraper-facing product command:

```bash
cargo run -- product "https://www.iherb.com/pr/california-gold-nutrition-gold-c-powder-usp-grade-vitamin-c-1-000-mg-8-81-oz-250-g/59561" --json --profile-dir ./runtime/chrome-profile --delay 10000
```

Debug/manual session recovery:

```bash
cargo run -- --debug product "https://www.iherb.com/pr/california-gold-nutrition-gold-c-powder-usp-grade-vitamin-c-1-000-mg-8-81-oz-250-g/59561"
```

Future scraper-friendly target:

```bash
iherb-cli product "$url" --json --profile-dir ./runtime/chrome-profile --delay 10000
```
