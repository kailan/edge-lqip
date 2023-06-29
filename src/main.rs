use fastly::cache::core::CacheKey;
use fastly::http::{header, Method};
use fastly::{mime, Error, Request, Response};
use image::io::Reader as ImageReader;
use image::GenericImageView;
use std::io::{Cursor, Write};
use std::time::Duration;
use thumbhash::rgba_to_thumb_hash;

const CONTENT_BACKEND: &str = "content_backend";

const CACHE_DURATION_SECS: u64 = 31536000;

// Generate a LQIP (low-quality image placeholder) from a given image
fn lqip_generator(mut req: Request) -> Result<Response, Error> {
    // Strip /lqip from the path.
    let image_path = req.get_path()[5..].to_owned();

    // Store a value to indicate whether the image was found in the cache.
    let mut x_cache = "MISS";

    // Check the cache for a previously generated LQIP.
    let thumb_hash = if let Some(entry) =
        fastly::cache::core::lookup(CacheKey::copy_from_slice(image_path.as_bytes()))
            .execute()
            .unwrap()
    {
        println!("cache hit");
        x_cache = "HIT";
        entry.to_stream().unwrap().into_bytes()
    } else {
        req.set_path(&image_path);

        // Apply any origin-specific transformations to the request.
        req.set_query_str(format!("{}&q=1", &req.get_query_str().unwrap_or("?")));

        // Send the request for the image to the content backend.
        let img_response = req.send(CONTENT_BACKEND)?;

        // Load the entire image into memory.
        let img_bytes = img_response.into_body_bytes();

        // Decode the image data from the raw bytes.
        let img = ImageReader::new(Cursor::new(img_bytes))
            .with_guessed_format()?
            .decode()?;

        // Get the image dimensions.
        let (width, height) = &img.dimensions();

        println!("b4 width {} height {}", width, height);

        // If the image is over 100px in either dimension, resize while maintaining aspect ratio.
        let img = if width > height && width > &100 {
            img.resize(
                100,
                (100f64 * (*height as f64) / (*width as f64)).floor() as u32,
                image::imageops::FilterType::Nearest,
            )
        } else if height > width && height > &100 {
            img.resize(
                (100f64 * (*width as f64) / (*height as f64)).floor() as u32,
                100,
                image::imageops::FilterType::Nearest,
            )
        } else {
            img
        };

        // Get the updated image dimensions.
        let (width, height) = &img.dimensions();

        println!("then width {} height {}", width, height);

        // Generate a thumbhash.
        let thumb_hash = rgba_to_thumb_hash(
            (*width).try_into().unwrap(),
            (*height).try_into().unwrap(),
            &img.to_rgba8().into_vec(),
        );

        let mut writer = fastly::cache::core::insert(
            CacheKey::copy_from_slice(image_path.as_bytes()),
            Duration::from_secs(CACHE_DURATION_SECS),
        )
        .known_length(thumb_hash.len() as u64)
        .execute()
        .unwrap();
        writer.write_all(&thumb_hash).unwrap();
        writer.finish().unwrap();

        thumb_hash
    };

    // Respond with the thumbhash.
    Ok(Response::from_body(thumb_hash)
        .with_content_type(mime::APPLICATION_OCTET_STREAM)
        .with_header(header::CACHE_CONTROL, format!("public, max-age={}, immutable", CACHE_DURATION_SECS))
        .with_header("X-Cache", x_cache))
}

#[fastly::main]
fn main(req: Request) -> Result<Response, Error> {
    // Pattern match on the request method and path.
    match (req.get_method(), req.get_path()) {
        // If the request is a `GET` to the `/lqip/*` path, generate a LQIP.
        (&Method::GET, path) if path.starts_with("/lqip/") => lqip_generator(req),
        // If the request is a `GET` to the `/` path, serve a HTML page.
        (&Method::GET, "/") => Ok(Response::new().with_body_text_html(include_str!("index.html"))),
        // Forward all other requests to the content backend.
        _ => Ok(req.send(CONTENT_BACKEND)?),
    }
}
