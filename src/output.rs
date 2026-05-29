use crate::cli::Section;
use crate::model::{ProductDetail, SearchResult};
use serde::Serialize;
use serde_json::json;
use std::time::SystemTime;

#[derive(Serialize)]
struct SearchProductJson<'a> {
    product_id: &'a str,
    product_code: Option<&'a str>,
    product_url: &'a str,
    name: &'a str,
    brand: &'a str,
}

#[derive(Serialize)]
struct ProductJson<'a> {
    product_id: &'a str,
    product_code: Option<&'a str>,
    product_url: &'a str,
    name: &'a str,
    brand: &'a str,
    image_url: Option<&'a str>,
    image_urls: &'a [String],
    category_breadcrumb: Option<&'a Vec<String>>,
    certifications_and_diet: &'a [String],
    description: Option<&'a str>,
    suggested_use: Option<&'a str>,
    other_ingredients: Option<&'a str>,
    supplement_facts: Option<&'a crate::model::SupplementFacts>,
    warnings: Option<&'a str>,
    upc: Option<&'a str>,
}

pub fn format_search_results(result: &SearchResult) -> String {
    let mut out = String::new();

    let total_str = match result.total_results {
        Some(total) => format!("{}+", format_number(total)),
        None => "?".to_string(),
    };
    let showing = result.products.len();
    out.push_str(&format!(
        "## Search results for \"{}\" (showing {} of {})\n\n",
        result.query, showing, total_str
    ));

    for (i, product) in result.products.iter().enumerate() {
        out.push_str(&format!("### {}. {}\n", i + 1, product.name));
        out.push_str(&format!("- **Brand:** {}\n", product.brand));
        if let Some(ref code) = product.product_code {
            out.push_str(&format!("- **Product Code:** {}\n", code));
        }

        let price_str = format_price(
            product.price,
            product.original_price.as_ref(),
            &product.currency,
        );
        out.push_str(&format!("- **Price:** {}\n", price_str));

        if let (Some(rating), Some(count)) = (product.rating, product.review_count) {
            out.push_str(&format!(
                "- **Rating:** {:.1}/5 ({} reviews)\n",
                rating,
                format_number(count)
            ));
        }

        out.push_str(&format!("- **ID:** {}\n", product.product_id));
        out.push_str(&format!("- **URL:** {}\n", product.product_url));

        if i < result.products.len() - 1 {
            out.push_str("\n---\n\n");
        }
    }

    out
}

pub fn format_search_json(result: &SearchResult) -> serde_json::Result<String> {
    let products: Vec<_> = result
        .products
        .iter()
        .map(|product| SearchProductJson {
            product_id: &product.product_id,
            product_code: product.product_code.as_deref(),
            product_url: &product.product_url,
            name: &product.name,
            brand: &product.brand,
        })
        .collect();

    serde_json::to_string_pretty(&json!({
        "ok": true,
        "data": {
            "query": &result.query,
            "total_results": result.total_results,
            "products": products,
        }
    }))
}

pub fn format_product_json(product: &ProductDetail) -> serde_json::Result<String> {
    let certifications_and_diet = product
        .key_info
        .as_ref()
        .map(|info| info.certifications_and_diet.as_slice())
        .unwrap_or(&[]);

    let data = ProductJson {
        product_id: &product.product_id,
        product_code: product.product_code.as_deref(),
        product_url: &product.product_url,
        name: &product.name,
        brand: &product.brand,
        image_url: product.image_url.as_deref(),
        image_urls: &product.image_urls,
        category_breadcrumb: product.category_breadcrumb.as_ref(),
        certifications_and_diet,
        description: product.description.as_deref(),
        suggested_use: product.suggested_use.as_deref(),
        other_ingredients: product.ingredients.as_deref(),
        supplement_facts: product.supplement_facts.as_ref(),
        warnings: product.warnings.as_deref(),
        upc: product.upc.as_deref(),
    };

    serde_json::to_string_pretty(&json!({
        "ok": true,
        "data": data,
    }))
}

pub fn format_error_json(error_type: &str, message: &str) -> String {
    serde_json::to_string_pretty(&json!({
        "ok": false,
        "error_type": error_type,
        "message": message,
    }))
    .unwrap_or_else(|_| {
        format!(
            r#"{{"ok":false,"error_type":"{}","message":"{}"}}"#,
            error_type, message
        )
    })
}

pub fn format_product_detail(product: &ProductDetail, section: Option<Section>) -> String {
    let mut out = String::new();

    let sections: &[Section] = match section {
        Some(s) => &[s],
        None => Section::ALL,
    };

    if section.is_none() {
        out.push_str(&format!("# {}\n\n", product.name));
    }

    for sec in sections {
        match sec {
            Section::Overview => format_overview(product, &mut out),
            Section::Description => format_description(product, &mut out),
            Section::Nutrition => format_nutrition(product, &mut out),
            Section::Ingredients => {
                // When explicitly requesting ingredients, show supplement facts
                // first (active ingredients) then other ingredients — matching
                // how supplement labels read and what users expect from "what's in it?"
                if section.is_some() {
                    format_nutrition(product, &mut out);
                }
                format_ingredients(product, &mut out);
            }
            Section::SuggestedUse => format_suggested_use(product, &mut out),
            Section::Warnings => format_warnings(product, &mut out),
            Section::Reviews => format_reviews(product, &mut out),
        }
    }

    if out.is_empty() {
        if let Some(sec) = section {
            out.push_str(&format!(
                "No {} data available for this product.\n",
                sec.label()
            ));
        }
    }

    out
}

fn format_overview(product: &ProductDetail, out: &mut String) {
    out.push_str("## Overview\n");
    out.push_str(&format!("- **Brand:** {}\n", product.brand));

    let price_str = format_price(
        product.price,
        product.original_price.as_ref(),
        &product.currency,
    );
    out.push_str(&format!("- **Price:** {}\n", price_str));

    if let (Some(rating), Some(count)) = (product.rating, product.review_count) {
        out.push_str(&format!(
            "- **Rating:** {:.1}/5 ({} reviews)\n",
            rating,
            format_number(count)
        ));
    }

    let stock_str = if product.in_stock {
        "In Stock"
    } else {
        "Out of Stock"
    };
    out.push_str(&format!("- **Availability:** {}\n", stock_str));

    if let Some(ref code) = product.product_code {
        out.push_str(&format!("- **Product Code:** {}\n", code));
    }
    if let Some(ref weight) = product.shipping_weight {
        out.push_str(&format!("- **Shipping Weight:** {}\n", weight));
    }
    out.push('\n');
}

fn format_description(product: &ProductDetail, out: &mut String) {
    if let Some(ref desc) = product.description {
        out.push_str("## Description\n");
        out.push_str(desc);
        out.push_str("\n\n");
    }
}

fn format_nutrition(product: &ProductDetail, out: &mut String) {
    let facts = match product.supplement_facts {
        Some(ref f) => f,
        None => return,
    };
    out.push_str("## Supplement Facts\n");
    if !facts.nutrients.is_empty() {
        out.push_str("| Nutrient | Amount | % Daily Value |\n");
        out.push_str("|---|---|---|\n");
        for nutrient in &facts.nutrients {
            let dv = nutrient.daily_value.as_deref().unwrap_or("");
            out.push_str(&format!(
                "| {} | {} | {} |\n",
                nutrient.name, nutrient.amount, dv
            ));
        }
        out.push('\n');
    }
    if let Some(ref size) = facts.serving_size {
        out.push_str(&format!("- **Serving Size:** {}\n", size));
    }
    if let Some(ref servings) = facts.servings_per_container {
        out.push_str(&format!("- **Servings Per Container:** {}\n", servings));
    }
    out.push('\n');
}

fn format_ingredients(product: &ProductDetail, out: &mut String) {
    if let Some(ref ingredients) = product.ingredients {
        out.push_str("## Other Ingredients\n");
        out.push_str(ingredients);
        out.push_str("\n\n");
    }
}

fn format_suggested_use(product: &ProductDetail, out: &mut String) {
    if let Some(ref usage) = product.suggested_use {
        out.push_str("## Suggested Use\n");
        out.push_str(usage);
        out.push_str("\n\n");
    }
}

fn format_warnings(product: &ProductDetail, out: &mut String) {
    if let Some(ref warnings) = product.warnings {
        out.push_str("## Warnings\n");
        out.push_str(warnings);
        out.push_str("\n\n");
    }
}

fn format_reviews(product: &ProductDetail, out: &mut String) {
    let dist = match product.review_distribution {
        Some(ref d) => d,
        None => return,
    };
    out.push_str("## Reviews\n");
    if let (Some(rating), Some(count)) = (product.rating, product.review_count) {
        out.push_str(&format!("- **Average:** {:.1}/5\n", rating));
        out.push_str(&format!("- **Total:** {} reviews\n", format_number(count)));
    }
    if let Some(pct) = dist.five_star {
        out.push_str(&format!("- 5 stars: {:.0}%\n", pct));
    }
    if let Some(pct) = dist.four_star {
        out.push_str(&format!("- 4 stars: {:.0}%\n", pct));
    }
    if let Some(pct) = dist.three_star {
        out.push_str(&format!("- 3 stars: {:.0}%\n", pct));
    }
    if let Some(pct) = dist.two_star {
        out.push_str(&format!("- 2 stars: {:.0}%\n", pct));
    }
    if let Some(pct) = dist.one_star {
        out.push_str(&format!("- 1 star: {:.0}%\n", pct));
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProductDetail, SupplementFacts};

    #[test]
    fn product_json_omits_dynamic_commerce_fields() {
        let product = ProductDetail {
            name: "Name".to_string(),
            brand: "Brand".to_string(),
            price: 12.34,
            original_price: Some(15.0),
            currency: "USD".to_string(),
            rating: Some(4.8),
            review_count: Some(100),
            product_url: "https://www.iherb.com/pr/example/1".to_string(),
            product_id: "1".to_string(),
            image_url: None,
            image_urls: Vec::new(),
            in_stock: true,
            description: None,
            product_code: Some("ABC-123".to_string()),
            upc: None,
            ingredients: Some("Main Ingredients\nA\n\nOther Ingredients\nB".to_string()),
            supplement_facts: Some(SupplementFacts {
                serving_size: Some("1 capsule".to_string()),
                servings_per_container: Some("30".to_string()),
                nutrients: Vec::new(),
            }),
            suggested_use: None,
            warnings: None,
            shipping_weight: None,
            category_breadcrumb: None,
            key_info: None,
            review_distribution: None,
        };

        let json = format_product_json(&product).expect("json should format");
        assert!(json.contains("\"other_ingredients\""));
        assert!(!json.contains("\"price\""));
        assert!(!json.contains("\"rating\""));
        assert!(!json.contains("\"review_count\""));
        assert!(!json.contains("\"in_stock\""));
        assert!(!json.contains("\"currency\""));
        assert!(!json.contains("\"shipping_weight\""));
        assert!(!json.contains("\"key_info\""));
        assert!(!json.contains("\"country_of_origin\""));
        assert!(json.contains("\"certifications_and_diet\""));
    }
}

fn format_price(price: f64, original: Option<&f64>, currency: &str) -> String {
    let symbol = match currency {
        "USD" => "$",
        "CHF" => "CHF ",
        "EUR" => "€",
        "GBP" => "£",
        _ => currency,
    };

    match original {
        Some(orig) if *orig > price => {
            let discount = ((*orig - price) / *orig * 100.0).round() as u32;
            format!(
                "{}{:.2} ~~{}{:.2}~~ ({}% off)",
                symbol, price, symbol, orig, discount
            )
        }
        _ => format!("{}{:.2}", symbol, price),
    }
}

pub fn format_cached_at(cached_at: SystemTime) -> String {
    let duration = cached_at
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs() as i64;

    // Simple date formatting without extra dependencies
    let days = secs / 86400;
    let remaining = secs % 86400;
    let hours = remaining / 3600;
    let minutes = (remaining % 3600) / 60;

    // Calculate year/month/day from epoch days
    let mut y = 1970i64;
    let mut d = days;
    loop {
        let days_in_year = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            366
        } else {
            365
        };
        if d < days_in_year {
            break;
        }
        d -= days_in_year;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut m = 0usize;
    for (i, &md) in month_days.iter().enumerate() {
        if d < md {
            m = i;
            break;
        }
        d -= md;
    }

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC",
        y,
        m + 1,
        d + 1,
        hours,
        minutes
    )
}

fn format_number(n: u32) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }
    result.chars().rev().collect()
}
