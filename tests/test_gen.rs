use bytes::Bytes;
use http_body::Body;
use http_body_util::combinators::BoxBody;

pub struct MyQuery {
    pub name: Option<String>,
    pub age: Option<u32>,
}

impl MyQuery {
    pub fn build_http_request(self) -> http::Request<BoxBody<Bytes, std::convert::Infallible>> {
        let mut req = http::Request::builder()
            .method("GET")
            .uri("http://example.com/api/resource");

        let mut query_params = vec![];
        if let Some(name) = self.name {
            query_params.push(format!("name={}", name));
        }
        if let Some(age) = self.age {
            query_params.push(format!("age={}", age));
        }

        if !query_params.is_empty() {
            let query_string = query_params.join("&");
            let uri = format!("http://example.com/api/resource?{}", query_string);
            *req.uri_mut() = uri.parse().unwrap();
        }

        req.body(BoxBody::new(http_body_util::Empty::<
            Bytes,
            std::convert::Infallible,
        >::new()))
            .unwrap()
    }
}
