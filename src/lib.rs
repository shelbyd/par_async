extern crate proc_macro;
use proc_macro::TokenStream;

use proc_macro2::Span;
use quote::quote;
use syn::fold::{self, Fold};
use syn::visit::{self, Visit};
use syn::{parse_macro_input, parse_quote, Expr, ItemFn};

#[derive(Default)]
struct ParAwait {
    assigns: Vec<(syn::Pat, syn::ExprAwait)>,
}

impl ParAwait {
    fn include(&self, stmt: &syn::Stmt) -> bool {
        match stmt {
            syn::Stmt::Local(local) => self.assigns.iter().all(|(pat, _)| pat != &local.pat),
            _ => true,
        }
    }

    fn join(&self) -> syn::Stmt {
        let tys = (0..self.assigns.len())
            .map(|i| syn::Ident::new(&format!("F{}", i), Span::call_site()))
            .collect::<Vec<_>>();
        let ns = (0..self.assigns.len())
            .map(syn::Index::from)
            .collect::<Vec<_>>();

        parse_quote!(
            fn join<#(#tys),*>(tuple: (#(#tys),*)) ->
                impl ::core::future::Future<Output = (#(#tys::Output),*)>
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
                    tuple: (#(Waiting<#tys>),*),
                }

                impl<#(#tys),*> Future for Join<#(#tys),*>
                where
                    #(#tys: Future,)*
                {
                    type Output = (#(#tys::Output),*);

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
                                }),*
                            )
                        };
                        Poll::Ready((#(result.#ns.take().unwrap()),*))
                    }
                }

                Join {
                    tuple: (#(Waiting::Future(Box::pin(tuple.#ns))),*),
                }
            }
        )
    }

    fn destructure(&self) -> syn::Stmt {
        let pats = self.assigns.iter().map(|(p, _)| p);
        let exprs = self.assigns.iter().map(|(_, e)| e).map(|e| &*e.base);

        parse_quote!(let (#(#pats),*) = join((#(#exprs),*)).await;)
    }
}

impl<'ast> Visit<'ast> for ParAwait {
    fn visit_local(&mut self, node: &'ast syn::Local) {
        let await_expr = match node.init.as_ref().unwrap().1.as_ref() {
            Expr::Await(aw) => aw,
            _ => unimplemented!(),
        };

        self.assigns.push((node.pat.clone(), await_expr.clone()));

        visit::visit_local(self, node);
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

    fn fold_block(&mut self, node: syn::Block) -> syn::Block {
        let inner = fold::fold_block(self, node);

        let join = self.join();
        let destructure = self.destructure();
        let remaining_block = inner.stmts.into_iter().filter(|stmt| self.include(stmt));

        syn::Block {
            stmts: vec![join, destructure]
                .into_iter()
                .chain(remaining_block)
                .collect(),
            ..inner
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
    out
}
