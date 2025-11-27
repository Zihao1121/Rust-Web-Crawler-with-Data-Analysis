use anyhow::{Context, Result};
use reqwest::Client;
use scraper::{ElementRef, Html, Selector};
use tokio::sync::Semaphore;
use std::sync::Arc;
fn clean_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extract_title(item: &ElementRef, title_sels: &[Selector], img_alt_sel: &Selector) -> Option<String> {
    // 1) try common title DOMs
    for sel in title_sels {
        if let Some(el) = item.select(sel).next() {
            let t = clean_text(&el.text().collect::<String>());
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    // 2) fallback: image alt is often the title
    if let Some(img) = item.select(img_alt_sel).next() {
        if let Some(alt) = img.value().attr("alt") {
            let t = clean_text(alt);
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}
#[derive(Debug)]
struct Detail {
    rating_text: String,   // e.g. "4.6 out of 5 stars"
    review_count: String,  // e.g. "12,345 ratings"
}
fn extract_link(item: &ElementRef, link_sels: &[Selector]) -> Option<String> {
    for sel in link_sels {
        if let Some(a) = item.select(sel).next() {
            if let Some(href) = a.value().attr("href") {
                return Some(if href.starts_with("http") {
                    href.to_string()
                } else {
                    format!("https://www.amazon.com{}", href)
                });
            }
        }
    }
    None
}
async fn fetch_detail(client: &Client, url: &str) -> Result<Detail> {
    let res = client
        .get(url)
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
        .with_context(|| format!("request detail failed: {url}"))?;

    let body = res.text().await.context("read detail body failed")?;
    let doc = Html::parse_document(&body);

    let rating_sel = Selector::parse("span.a-icon-alt").unwrap();
    let review_count_sel = Selector::parse("#acrCustomerReviewText").unwrap();

    let rating_text = doc
        .select(&rating_sel)
        .next()
        .map(|e| clean_text(&e.text().collect::<String>()))
        .unwrap_or_else(|| "N/A".to_string());

    let review_count = doc
        .select(&review_count_sel)
        .next()
        .map(|e| clean_text(&e.text().collect::<String>()))
        .unwrap_or_else(|| "N/A".to_string());

    Ok(Detail { rating_text, review_count })
}
#[tokio::main]
async fn main() -> Result<()> {
    let url = "https://www.amazon.com/s?k=laptop";

    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36")
        .build()
        .context("build client failed")?;

    let res = client
        .get(url)
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
        .context("request failed")?;

    println!("Status: {}", res.status());
    println!("Final URL: {}", res.url());

    let body = res.text().await.context("read body failed")?;
    std::fs::write("amazon_debug.html", &body).ok();

    let doc = Html::parse_document(&body);

    // Each search result card
    let item_sel = Selector::parse(r#"div[data-component-type="s-search-result"]"#).unwrap();

    // Title fallbacks
    let title_sels = vec![
        Selector::parse("h2 a span").unwrap(),
        Selector::parse(r#"a.a-link-normal.s-line-clamp-2 span"#).unwrap(),
        Selector::parse(r#"span.a-size-base-plus.a-color-base.a-text-normal"#).unwrap(),
        Selector::parse(r#"span.a-size-medium.a-color-base.a-text-normal"#).unwrap(),
    ];
    let img_alt_sel = Selector::parse("img.s-image").unwrap();

    // Link fallbacks
    let link_sels = vec![
        Selector::parse("h2 a").unwrap(),
        Selector::parse(r#"a.a-link-normal.s-no-outline"#).unwrap(),
        Selector::parse(r#"a.a-link-normal[href*="/dp/"]"#).unwrap(),
    ];

    // Price (full string like "$563.68")
    let price_sel = Selector::parse("span.a-price span.a-offscreen").unwrap();

    println!("\n📦 Amazon Search Results (skip cards without title):\n");

    let mut shown = 0usize;
    for item in doc.select(&item_sel) {
        // ✅ skip cards without title
        let Some(title) = extract_title(&item, &title_sels, &img_alt_sel) else {
            continue;
        };

        let price = item
            .select(&price_sel)
            .next()
            .map(|e| clean_text(&e.text().collect::<String>()))
            .unwrap_or_else(|| "N/A".to_string());

        let link = extract_link(&item, &link_sels).unwrap_or_else(|| "N/A".to_string());

        shown += 1;
        println!("{:02}. {} — {}", shown, title, price);
        println!("    {}", link);

        if shown >= 10 {
            break;
        }
        let sem = Arc::new(Semaphore::new(3)); // 并发限制：最多同时开 3 个详情页请求
        let permit = sem.clone().acquire_owned().await?;
        let detail = fetch_detail(&client, &link).await;
        drop(permit);

        match detail {
            Ok(d) => {
                println!("    rating: {}", d.rating_text);
                println!("    reviews: {}", d.review_count);
            }
            Err(e) => {
                println!("    detail fetch failed: {}", e);
            }
        }
    }

    if shown == 0 {
        println!("No titled cards found. Open amazon_debug.html and inspect a result card to update selectors.");
    }



    Ok(())
}
