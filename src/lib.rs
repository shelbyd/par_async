extern crate proc_macro;
use proc_macro::TokenStream;

use proc_macro2::Span;
use quote::quote;
use syn::fold::{self, Fold};
use syn::visit::{self, Visit};
use syn::{parse_macro_input, parse_quote, ItemFn};

#[derive(Default)]
struct ParAwait {
    awaits: Vec<syn::ExprAwait>,
}

impl ParAwait {
    fn join(&self) -> syn::Stmt {
        let tys = (0..self.awaits.len())
            .map(|i| syn::Ident::new(&format!("F{}", i), Span::call_site()))
            .collect::<Vec<_>>();
        let ns = (0..self.awaits.len())
            .map(syn::Index::from)
            .collect::<Vec<_>>();

        parse_quote!(
            fn join<#(#tys),*>(tuple: (#(#tys,)*)) ->
                impl ::core::future::Future<Output = (#(#tys::Output,)*)>
            where
                #(#tys: ::core::future::Future,)*
            {
                use ::core::{future::Future, pin::Pin, task::{self, Poll}};

                enum Waiting<F: Future> {
                    Future(F),
                    Ready(Option<F::Output>),
                }

                impl<F: Future> Waiting<F> {
                    fn poll(self: Pin<&mut Self>, cx: &mut task::Context) {
                        let this = unsafe { self.get_unchecked_mut() };
                        if let Waiting::Future(pin) = this {
                            let pin = unsafe { Pin::new_unchecked(pin) };
                            if let Poll::Ready(v) = pin.poll(cx) {
                                *this = Waiting::Ready(Some(v));
                            }
                        }
                    }
                }

                struct Join<#(#tys),*>
                where
                    #(#tys: Future,)*
                {
                    tuple: (#(Waiting<#tys>,)*),
                }

                impl<#(#tys),*> Future for Join<#(#tys),*>
                where
                    #(#tys: Future,)*
                {
                    type Output = (#(#tys::Output,)*);

                    fn poll(
                        self: Pin<&mut Self>,
                        cx: &mut task::Context,
                    ) -> Poll<Self::Output> {
                        let tuple = unsafe { &mut self.get_unchecked_mut().tuple };

                        #({ unsafe { Pin::new_unchecked(&mut tuple.#ns) }.poll(cx); })*

                        let result = {
                            (
                                #(match &mut tuple.#ns {
                                    Waiting::Ready(v) => v,
                                    Waiting::Future(_) => return Poll::Pending,
                                },)*
                            )
                        };
                        Poll::Ready((#(result.#ns.take().unwrap(),)*))
                    }
                }

                Join {
                    tuple: (#(Waiting::Future(Box::pin(tuple.#ns)),)*),
                }
            }
        )
    }

    fn futures(&self) -> impl Iterator<Item = syn::Stmt> + '_ {
        self.awaits.iter().map(move |aw| {
            let ident = self.future_ident(aw);
            let expr = &*aw.base;
            parse_quote!(let #ident = #expr;)
        })
    }

    fn future_ident(&self, node: &syn::ExprAwait) -> syn::Ident {
        self.awaits
            .iter()
            .enumerate()
            .find(|(_, aw)| *aw == node)
            .map(|(i, _)| syn::Ident::new(&format!("__par_async_future{}", i), Span::call_site()))
            .expect("Could not find expr in known awaits")
    }

    fn values(&self) -> syn::Stmt {
        let value_idents = self.awaits.iter().map(|aw| self.value_ident(aw));
        let future_idents = self.awaits.iter().map(|aw| self.future_ident(aw));
        parse_quote!(let (#(#value_idents,)*) = join((#(#future_idents,)*)).await;)
    }

    fn value_ident(&self, node: &syn::ExprAwait) -> syn::Ident {
        self.awaits
            .iter()
            .enumerate()
            .find(|(_, aw)| *aw == node)
            .map(|(i, _)| syn::Ident::new(&format!("__par_async_value{}", i), Span::call_site()))
            .expect("Could not find expr in known awaits")
    }
}

impl<'ast> Visit<'ast> for ParAwait {
    fn visit_expr_await(&mut self, node: &'ast syn::ExprAwait) {
        self.awaits.push(node.clone());

        visit::visit_expr_await(self, node);
    }
}

impl Fold for ParAwait {
    fn fold_item_fn(&mut self, node: syn::ItemFn) -> syn::ItemFn {
        if node.sig.asyncness.is_some() {
            fold::fold_item_fn(self, node)
        } else {
            node
        }
    }

    fn fold_expr(&mut self, node: syn::Expr) -> syn::Expr {
        match node {
            syn::Expr::Await(aw) => {
                let value_ident = self.value_ident(&aw);
                parse_quote!(#value_ident)
            },
            other => fold::fold_expr(self, other),
        }
    }

    fn fold_block(&mut self, node: syn::Block) -> syn::Block {
        use std::iter;

        let replaced = fold::fold_block(self, node);

        let join = self.join();
        let futures = self.futures();
        let values = self.values();

        syn::Block {
            stmts: iter::once(join)
                .chain(futures)
                .chain(iter::once(values))
                .into_iter()
                .chain(replaced.stmts)
                .collect(),
            ..replaced
        }
    }
}

#[proc_macro_attribute]
pub fn par_async(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);

    let mut par_await = ParAwait::default();
    par_await.visit_item_fn(&input);

    let output = par_await.fold_item_fn(input);
    let out = TokenStream::from(quote!(#output));
    eprintln!("{}", out);
    out
}
