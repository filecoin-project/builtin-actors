use proc_macro::TokenStream;
use quote::quote;

#[proc_macro_attribute]
pub fn discoverable_test(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input_fn = syn::parse_macro_input!(item as syn::ItemFn);
    let fn_name = &input_fn.sig.ident;
    let registry_code = quote! {
        #input_fn
        #[ctor::ctor]
        fn register_test() {
            TEST_REGISTRY.lock().unwrap().insert(stringify!(#fn_name).to_string(), #fn_name);
        }
    };
    registry_code.into()
}
