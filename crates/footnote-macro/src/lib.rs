//! Derive macro for the footnote crate.

use std::collections::{HashMap, HashSet};

use heck::ToPascalCase;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, PathArguments, parse_macro_input};

#[proc_macro_derive(Footnote, attributes(patch))]
pub fn derive_footnote(input: TokenStream) -> TokenStream {
    // let input = proc_macro2::TokenStream::from(input);
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;
    let origin = name.clone();
    let enum_name = format_ident!("{}Field", name);
    let hash_name = format_ident!("{}Note", name);
    let patch_name = format_ident!("{}Patch", name);

    let fields = match input.data {
        Data::Struct(syn::DataStruct {
            fields: Fields::Named(fields_named),
            ..
        }) => fields_named.named,
        _ => panic!("Not supported!"),
    };

    let fields: Vec<_> = fields
        .into_iter()
        .filter(|field| {
            !field.attrs.iter().any(|attr| {
                attr.path().is_ident("patch") && {
                    let mut skip = false;
                    let _ = attr.parse_nested_meta(|meta| {
                        skip = meta.path.is_ident("skip");
                        Ok(())
                    });
                    skip
                }
            })
        })
        .collect();

    let nested: HashSet<_> = fields
        .iter()
        .filter(|field| {
            field.attrs.iter().any(|attr| {
                attr.path().is_ident("patch") && {
                    let mut nesting = false;
                    let _ = attr.parse_nested_meta(|meta| {
                        nesting = meta.path.is_ident("nesting");
                        Ok(())
                    });
                    nesting
                }
            })
        })
        .filter_map(|field| field.ident.clone())
        .collect();

    let variants = fields.iter().map(|field| {
        let name = field.ident.as_ref().unwrap();
        let name = format_ident!("{}", format!("{name}").to_pascal_case());
        quote! {
            #name
        }
    });

    let mut nest_types = HashMap::new();
    let fieldy = fields.iter().map(|field| {
        let name = field.ident.as_ref().unwrap();
        if nested.contains(name) {
            let sub_ty = match &field.ty {
                // syn::Type::Verbatim(ty) => ty.clone(),
                syn::Type::Path(tp) => {
                    let Some(seg) = tp.path.segments.last() else {
                        panic!("Empty type path")
                    };

                    if !matches!(seg.arguments, PathArguments::None) {
                        panic!("Parameterized types not supported");
                    }

                    format_ident!("{}Note", seg.ident)
                }
                unk => panic!("Unsupported nest type: {}", quote!(#unk)),
            };

            nest_types.insert(name.clone(), sub_ty.clone());

            quote! {
                #name: #sub_ty
            }
        } else {
            quote! {
                #name: u64
            }
        }
    });

    let hashy = fields.iter().map(|field| {
        let name = field.ident.as_ref().unwrap();
        if nested.contains(name) {
            quote! {
                #name: self.#name.get_note()
            }
        } else {
            // let hashed = calc_hash(quote!(self.#name));

            quote! {
                #name: #origin::calc_hash(&self.#name)
            }
        }
    });

    let checky = fields.iter().map(|field| {
        let name = field.ident.as_ref().unwrap();
        if nested.contains(name) {
            let pascal = format_ident!("{}", format!("{name}").to_pascal_case());

            quote! {
                if !self.#name.check_note(&note.#name).is_empty()  {
                    changed.push(#enum_name::#pascal);
                }
            }
        } else {
            // let hashed = calc_hash(quote!(self.#name));
            let pascal = format_ident!("{}", format!("{name}").to_pascal_case());

            quote! {
                if note.#name != #origin::calc_hash(&self.#name) {
                    changed.push(#enum_name::#pascal);
                }
            }
        }
    });

    let patchy = fields.iter().map(|field| {
        let name = field.ident.as_ref().unwrap();
        if nested.contains(name) {
            quote! {
                #name: self.#name.make_patch(&note.#name)
            }
        } else {
            // let hashed = calc_hash(quote!(self.#name));

            quote! {
                #name: if note.#name == #origin::calc_hash(&self.#name) {
                    None
                } else {
                    Some(self.#name.clone())
                }
            }
        }
    });

    // TODO: use derives from patch(attribute(...)) instead of hard-coding
    let output: proc_macro2::TokenStream = quote! {
        #[derive(Debug, Default, Hash, PartialEq, Eq, Clone, serde::Serialize, serde::Deserialize)]
        pub struct #hash_name {
            #(#fieldy),*
        }

        #[derive(Debug, Hash, PartialEq, Eq, Clone, serde::Serialize, serde::Deserialize)]
        pub enum #enum_name {
            #(#variants),*
        }

        impl Footnote for #name {
            type Note = #hash_name;
            type Patch = #patch_name;
            type Field = #enum_name;

            fn get_note(&self) -> Self::Note {
                Self::Note {
                    #(#hashy),*
                }
            }

            fn check_note(&self, note: &Self::Note) -> Vec<Self::Field> {
                let mut changed = vec![];

                #(#checky)*

                changed
            }

            fn make_patch(&self, note: &Self::Note) -> Self::Patch {
                Self::Patch {
                    #(#patchy),*
                }
            }
        }
    };

    proc_macro::TokenStream::from(output)
}
