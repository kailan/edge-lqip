use blurhash::{decode, encode};
use fastly::http::{header, Method, StatusCode};
use fastly::{mime, Error, Request, Response};
use image::io::Reader as ImageReader;
use image::{DynamicImage, GenericImageView, ImageBuffer};
use std::io::Cursor;

const IMAGES_BACKEND: &str = "images_backend";
const IMAGES_HOST: &str = "images.unsplash.com";

// Generate a LQIP (low-quality image placeholder) from a given image
fn lqip_generator(mut req: Request) -> Result<Response, Error> {
    // Strip /lqip from the path.
    let image_path = &req.get_path()[5..].to_owned();
    req.set_path(image_path);

    // The `q` parameter is specific to the image backend used in our example (unsplash.com),
    // and is used to specify the image quality.
    // We don't need a high quality version of the image to generate a blurry placeholder!
    req.set_query_str(format!("{}&q=1", &req.get_query_str().unwrap_or_default()));

    // Retrieve a low-quality version of the image from the image backend.
    let img_bytes = req.send(IMAGES_BACKEND)?.take_body_bytes();

    // Decode the image.
    let img = ImageReader::new(Cursor::new(img_bytes))
        .with_guessed_format()?
        .decode()?;

    // Get the image dimensions.
    let (width, height) = &img.dimensions();

    // Generate a blurhash.
    let blurhash = encode(7, 6, *width, *height, &img.to_rgba8().into_vec());

    // Turn the blurhash into an array of pixels for our placeholder image.
    let pixels = decode(&blurhash, *width, *height, 1.0);

    // Generate the LQIP.
    let placeholder =
        DynamicImage::ImageRgba8(ImageBuffer::from_vec(*width, *height, pixels).unwrap());

    // Encode it as a JPEG. Quality = 30 is plenty for our purposes.
    let mut bytes: Vec<u8> = Vec::new();
    placeholder.write_to(&mut bytes, image::ImageOutputFormat::Jpeg(30))?;

    // Respond with the placeholder image and long cache directives.
    Ok(Response::from_body(bytes)
        .with_content_type(mime::IMAGE_JPEG)
        .with_header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .with_header("X-Blurhash", blurhash)
        .with_header("X-Width", format!("{}", width))
        .with_header("X-Height", format!("{}", height)))
}

#[fastly::main]
fn main(mut req: Request) -> Result<Response, Error> {
    // Set an override host header.
    req.set_header(header::HOST, IMAGES_HOST);
    // Pattern match on the request method and path.
    match (req.get_method(), req.get_path()) {
        // Demo set-up (see https://developer.fastly.com/solutions/demos)
        (&Method::GET, "/.well-known/fastly/demo-manifest") => {
            Ok(Response::new().with_body_text_plain(include_str!("demo-manifest")))
        }
        (&Method::GET, "/screenshot.jpg") => Ok(Response::new()
            .with_body_octet_stream(include_bytes!("screenshot.jpg"))
            .with_content_type(mime::IMAGE_PNG)
            .with_header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")),
        // If the request is a `GET` to the `/` path, serve a html page.
        (&Method::GET, "/") => Ok(Response::new().with_body_text_html(include_str!("index.html"))),
        // If the request is a `GET` to the `/lqip/*` path, generate a LQIP (low-quality image placeholder).
        (&Method::GET, path) if path.starts_with("/lqip/") => lqip_generator(req),
        // Forward other `GET` requests to the image backend.
        (&Method::GET, _) => {
            let mut beresp = req.send(IMAGES_BACKEND)?;
            beresp.set_header(header::CACHE_CONTROL, "public, max-age=31536000, immutable");
            Ok(beresp)
        }
        // Catch all other requests and return a 404.
        _ => Ok(Response::from_status(StatusCode::NOT_FOUND)),
    }
}
