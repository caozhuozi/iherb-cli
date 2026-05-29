---
name: iherb-agent
description: Use the local iherb-cli command-line tool to search iHerb products and extract scraper-friendly product metadata as JSON. Use when the user asks Codex to run iHerb searches, fetch an iHerb product detail page, inspect supplement facts, ingredients, suggested use, warnings, image URLs, category breadcrumbs, product identifiers, or produce machine-readable iHerb product/search JSON from this repository.
---

# iHerb Agent

## Build

The CLI binary is named `iherb-cli`.

```bash
cargo build
./target/debug/iherb-cli --help
```

During development, `cargo run -- ...` is usually enough and automatically builds before running.

## Browser Profile

Use the persisted Chrome profile for live iHerb runs:

```bash
--profile-dir ./runtime/chrome-profile
```

This keeps cookies, storefront preferences, and challenge recovery state between runs.

## Search Products

Search returns a compact product discovery JSON list.

```bash
cargo run -- --profile-dir ./runtime/chrome-profile --no-cache --json search 'calcium' --limit 20
```

Useful options:

- `--limit <n>`: maximum number of products to return
- `--sort relevance|price-asc|price-desc|rating|best-selling`
- `--category <slug>`: optional category filter

Search JSON includes product ID, product code, product URL, name, and brand.

Search JSON shape:

```json
{
  "ok": true,
  "data": {
    "query": "calcium",
    "total_results": 17827,
    "products": [
      {
        "product_id": "11574",
        "product_code": "CEN-27070",
        "product_url": "https://www.iherb.com/pr/21st-century-calcium-plus-d3-90-tablets/11574",
        "name": "21st Century, Calcium Plus D3, 90 Tablets",
        "brand": "21st Century"
      }
    ]
  }
}
```

## Product Details

Product accepts either a full iHerb URL or a numeric product ID.

```bash
cargo run -- --profile-dir ./runtime/chrome-profile --no-cache --json product 'https://www.iherb.com/pr/21st-century-calcium-plus-d3-90-tablets/11574'
```

Product JSON includes supplement metadata such as:

- product identifiers and URL
- name and brand
- image URL fields
- category breadcrumb
- certifications and diet labels
- description
- suggested use
- other ingredients
- supplement facts
- warnings
- UPC

Product JSON shape:

```json
{
  "ok": true,
  "data": {
    "product_id": "11574",
    "product_code": "CEN-27070",
    "product_url": "https://www.iherb.com/pr/21st-century-calcium-plus-d3-90-tablets/11574",
    "name": "21st Century, Calcium Plus D3, 90 Tablets",
    "brand": "21st Century",
    "image_url": "https://cloudinary.images-iherb.com/image/upload/f_auto,q_auto:eco/images/cen/cen27070/y/76.jpg",
    "image_urls": [
      "https://cloudinary.images-iherb.com/image/upload/f_auto,q_auto:eco/images/cen/cen27070/y/76.jpg"
    ],
    "category_breadcrumb": [
      "Supplements",
      "Minerals",
      "Calcium",
      "Calcium & Vitamin D"
    ],
    "certifications_and_diet": [
      "Gluten-free",
      "Soy-free"
    ],
    "description": "Product description text...",
    "suggested_use": "Suggested use text...",
    "other_ingredients": "Other ingredients text...",
    "supplement_facts": {
      "serving_size": "1 Tablet",
      "servings_per_container": null,
      "nutrients": [
        {
          "name": "Calcium (as Calcium Carbonate)",
          "amount": "1,000 mg",
          "daily_value": "77%"
        }
      ]
    },
    "warnings": "Warnings text...",
    "upc": "740985270707"
  }
}
```

## Output

Use `--json` when another scraper or agent will consume the output. Treat stdout as the machine-readable result.

For human inspection, omit `--json`:

```bash
cargo run -- --profile-dir ./runtime/chrome-profile product 11574
```

## Timing

Use `--timing` when debugging slow live runs:

```bash
cargo run -- --profile-dir ./runtime/chrome-profile --no-cache --json --timing product 11574
```

Timing logs are diagnostic output; the JSON result remains the primary output.

## Common Examples

Search calcium products:

```bash
cargo run -- --profile-dir ./runtime/chrome-profile --no-cache --json search 'calcium' --limit 20
```

Fetch a product detail page:

```bash
cargo run -- --profile-dir ./runtime/chrome-profile --no-cache --json product 11574
```

Fetch a full URL:

```bash
cargo run -- --profile-dir ./runtime/chrome-profile --no-cache --json product 'https://www.iherb.com/pr/21st-century-calcium-plus-d3-90-tablets/11574'
```
