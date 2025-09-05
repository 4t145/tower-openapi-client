use http::Method;
use oas3::spec;
use quote::{ToTokens, quote};
pub fn build_from_spec(spec: oas3::Spec) -> Result<(), spec::Error> {
    for (name, method, operation) in spec.operations() {
        let request_body = operation.request_body(&spec)?;
    }
    Ok(())
}

// pub fn build_schema_type(
//     spec: spec::Spec,
//     body: &spec::ObjectSchema,
// ) -> Result<syn::Item, spec::Error> {

//     Ok(())
// }
