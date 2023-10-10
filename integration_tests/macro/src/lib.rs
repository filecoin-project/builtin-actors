use proc_macro::TokenStream;
use quote::{format_ident, quote};

#[proc_macro_attribute]
pub fn exported_test(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input_fn = syn::parse_macro_input!(item as syn::ItemFn);
    let fn_name = &input_fn.sig.ident;

    // Generate a unique identifier for the registration function
    let register_fn_name = format_ident!("register_{}", fn_name);

    let registry_code = quote! {
        #input_fn
        #[ctor::ctor]
        fn #register_fn_name() {
            TEST_REGISTRY.lock().unwrap().insert(stringify!(#fn_name).to_string(), #fn_name);
        }
    };
    registry_code.into()
}
