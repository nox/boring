use boring::ex_data::Index;
use boring::ssl::{self, ClientHello, Ssl, SslContextBuilder};
use once_cell::sync::Lazy;
use std::future::Future;
use std::pin::Pin;
use std::task::{ready, Context, Poll};

type BoxSelectCertFuture = Pin<
    Box<
        dyn Future<Output = Result<BoxSelectCertFinish, AsyncSelectCertError>>
            + Send
            + Sync
            + 'static,
    >,
>;

type BoxSelectCertFinish =
    Box<dyn FnOnce(ClientHello<'_>) -> Result<(), AsyncSelectCertError> + 'static>;

pub(crate) static TASK_CONTEXT_INDEX: Lazy<Index<Ssl, usize>> =
    Lazy::new(|| Ssl::new_ex_index().unwrap());
pub(crate) static SELECT_CERT_FUTURE_INDEX: Lazy<Index<Ssl, BoxSelectCertFuture>> =
    Lazy::new(|| Ssl::new_ex_index().unwrap());

/// Extensions to [`SslContextBuilder`].
///
/// This trait provides additional methods to use async callbacks with boring.
pub trait SslContextBuilderExt: private::Sealed {
    /// Sets a callback that is called before most [`ClientHello`] processing
    /// and before the decision whether to resume a session is made. The
    /// callback may inspect the [`ClientHello`] and configure the connection.
    ///
    /// This method uses a function that returns a future whose output is
    /// itself a closure that will be passed [`ClientHello`] to configure
    /// the connection based on the computations done in the future.
    ///
    /// See [`SslContextBuilder::set_select_certificate_callback`] for the sync
    /// setter of this callback.
    fn set_async_select_certificate_callback<Init, Fut, Finish>(&mut self, callback: Init)
    where
        Init: Fn(&mut ClientHello<'_>) -> Result<Fut, AsyncSelectCertError> + Send + Sync + 'static,
        Fut: Future<Output = Result<Finish, AsyncSelectCertError>> + Send + Sync + 'static,
        Finish: FnOnce(ClientHello<'_>) -> Result<(), AsyncSelectCertError> + 'static;

    /// Sets a callback that is called before most [`ClientHello`] processing
    /// and before the decision whether to resume a session is made. The
    /// callback may inspect the [`ClientHello`] and configure the connection.
    ///
    /// This method uses a polling function.
    ///
    /// See [`SslContextBuilder::set_select_certificate_callback`] for the sync
    /// setter of this callback.
    fn set_polling_select_certificate_callback<F>(
        &mut self,
        callback: impl Fn(ClientHello<'_>, &mut Context<'_>) -> Poll<Result<(), AsyncSelectCertError>>
            + Send
            + Sync
            + 'static,
    );
}

impl SslContextBuilderExt for SslContextBuilder {
    fn set_async_select_certificate_callback<Init, Fut, Finish>(&mut self, callback: Init)
    where
        Init: Fn(&mut ClientHello<'_>) -> Result<Fut, AsyncSelectCertError> + Send + Sync + 'static,
        Fut: Future<Output = Result<Finish, AsyncSelectCertError>> + Send + Sync + 'static,
        Finish: FnOnce(ClientHello<'_>) -> Result<(), AsyncSelectCertError> + 'static,
    {
        self.set_select_certificate_callback(async_select_certificate_callback(callback))
    }

    fn set_polling_select_certificate_callback<F>(
        &mut self,
        callback: impl Fn(ClientHello<'_>, &mut Context<'_>) -> Poll<Result<(), AsyncSelectCertError>>
            + Send
            + Sync
            + 'static,
    ) {
        self.set_select_certificate_callback(polling_select_certificate_callback(callback));
    }
}

/// A fatal error to be returned from select certificate callbacks.
pub struct AsyncSelectCertError;

fn async_select_certificate_callback<Init, Fut, Finish>(
    callback: Init,
) -> impl Fn(ClientHello<'_>) -> Result<(), ssl::SelectCertError> + Send + Sync + 'static
where
    Init: Fn(&mut ClientHello<'_>) -> Result<Fut, AsyncSelectCertError> + Send + Sync + 'static,
    Fut: Future<Output = Result<Finish, AsyncSelectCertError>> + Send + Sync + 'static,
    Finish: FnOnce(ClientHello<'_>) -> Result<(), AsyncSelectCertError> + 'static,
{
    polling_select_certificate_callback(move |mut client_hello, cx| {
        let fut_result = match client_hello
            .ssl_mut()
            .ex_data_mut(*SELECT_CERT_FUTURE_INDEX)
        {
            Some(fut) => ready!(fut.as_mut().poll(cx)),
            None => {
                let fut = callback(&mut client_hello)?;
                let mut box_fut =
                    Box::pin(async move { Ok(Box::new(fut.await?) as BoxSelectCertFinish) })
                        as BoxSelectCertFuture;

                match box_fut.as_mut().poll(cx) {
                    Poll::Ready(fut_result) => fut_result,
                    Poll::Pending => {
                        client_hello
                            .ssl_mut()
                            .set_ex_data(*SELECT_CERT_FUTURE_INDEX, box_fut);

                        return Poll::Pending;
                    }
                }
            }
        };

        // NOTE(nox): For memory usage concerns, maybe we should implement
        // a way to remove the stored future from the `Ssl` value here?

        Poll::Ready(fut_result?(client_hello))
    })
}

fn polling_select_certificate_callback(
    callback: impl Fn(ClientHello<'_>, &mut Context<'_>) -> Poll<Result<(), AsyncSelectCertError>>
        + Send
        + Sync
        + 'static,
) -> impl Fn(ClientHello<'_>) -> Result<(), ssl::SelectCertError> + Send + Sync + 'static {
    move |client_hello| {
        let cx = unsafe {
            &mut *(*client_hello
                .ssl()
                .ex_data(*TASK_CONTEXT_INDEX)
                .expect("task context should be set") as *mut Context<'_>)
        };

        match callback(client_hello, cx) {
            Poll::Ready(Ok(())) => Ok(()),
            Poll::Ready(Err(AsyncSelectCertError)) => Err(ssl::SelectCertError::ERROR),
            Poll::Pending => Err(ssl::SelectCertError::RETRY),
        }
    }
}

mod private {
    pub trait Sealed {}
}

impl private::Sealed for SslContextBuilder {}
