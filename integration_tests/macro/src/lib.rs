use proc_macro::TokenStream;
use quote::{format_ident, quote};

/// The vm_test attribute is used to decorate tests that run on an implementation of the FVM (i.e.
/// taking vm_api::VM as an argument). Decorated tests are added to the global TEST_REGISTRY which
/// is exported for use in other environments.
/// TEST_REGISTRY acts as a single entry point for external crates/tooling to discover the suite of
/// builtin-actors' integration tests.
/// Test speed is an optional argument to the macro which must be a u8. Speed defaults to 0 indicating
/// a fast test, with value increasing for slower tests.
#[proc_macro_attribute]
pub fn vm_test(attr: TokenStream, item: TokenStream) -> TokenStream {
    // Try to parse the u8 argument
    let literal_arg: Result<syn::Lit, _> = syn::parse(attr.clone());

    // Determine the test speed based on the provided argument
    let test_category = if attr.is_empty() {
        0 // Default if not provided
    } else {
        match literal_arg {
            Ok(syn::Lit::Int(lit_int)) => {
                // Attempt to parse the integer
                match lit_int.base10_parse::<u8>() {
                    Ok(val) => val,
                    Err(_) => panic!("Test speed value is too large. Please use a u8."),
                }
            }
            _ => panic!("Invalid argument for test speed. Please provide a u8 value."),
        }
    };

    let input_fn = syn::parse_macro_input!(item as syn::ItemFn);
    let fn_name = &input_fn.sig.ident;

    // Generate a unique identifier for the registration function (unique within the module)
    let register_fn_name = format_ident!("register_{}", fn_name);

    let registry_code = quote! {
        #input_fn
        #[ctor::ctor]
        fn #register_fn_name() {
            // Registry key needs to be globally unique so we include module name
            let registry_key = concat!(module_path!(), "::", stringify!(#fn_name));
            crate::TEST_REGISTRY.lock().unwrap().insert(registry_key.to_string(), (#test_category, #fn_name));
        }
    };

    registry_code.into()
}
