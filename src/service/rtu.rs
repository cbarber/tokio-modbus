use crate::{
    client::Client,
    codec,
    frame::{rtu::*, *},
    slave::*,
};

use futures_util::{future, sink::SinkExt, stream::StreamExt};
use std::{
    future::Future,
    io::{Error, ErrorKind},
    pin::Pin,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::Framed;

pub(crate) fn connect_slave<T>(
    transport: T,
    slave: Slave,
) -> impl Future<Output = Result<Context<T>, Error>>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
{
    let framed = Framed::new(transport, codec::rtu::ClientCodec::default());

    let slave_id = slave.into();
    future::ok(Context {
        service: framed,
        slave_id,
    })
}

/// Modbus RTU client
pub(crate) struct Context<T: AsyncRead + AsyncWrite + Unpin + 'static> {
    service: Framed<T, codec::rtu::ClientCodec>,
    slave_id: SlaveId,
}

impl<T: AsyncRead + AsyncWrite + Unpin + 'static> Context<T> {
    fn next_request_adu<R>(&self, req: R, disconnect: bool) -> RequestAdu
    where
        R: Into<RequestPdu>,
    {
        let slave_id = self.slave_id;
        let hdr = Header { slave_id };
        let pdu = req.into();
        RequestAdu {
            hdr,
            pdu,
            disconnect,
        }
    }

    async fn call(&mut self, req: Request) -> Result<Option<Response>, Error> {
        let disconnect = req == Request::Disconnect;
        let req_adu = self.next_request_adu(req, disconnect);
        let req_hdr = req_adu.hdr;

        self.service.send(req_adu).await?;

        if self.slave_id == Slave::broadcast().0 {
            return Ok(None);
        }

        let res_adu = self
            .service
            .next()
            .await
            .ok_or_else(|| Error::new(ErrorKind::Other, "No response from request"))??;

        match res_adu.pdu {
            ResponsePdu(Ok(res)) => verify_response_header(req_hdr, res_adu.hdr).and(Ok(res)),
            ResponsePdu(Err(err)) => Err(Error::new(ErrorKind::Other, err)),
        }
    }
}

fn verify_response_header(req_hdr: Header, rsp_hdr: Header) -> Result<(), Error> {
    if req_hdr != rsp_hdr {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "Invalid response header: expected/request = {:?}, actual/response = {:?}",
                req_hdr, rsp_hdr
            ),
        ));
    }
    Ok(())
}

impl<T: AsyncRead + AsyncWrite + Unpin + 'static> SlaveContext for Context<T> {
    fn set_slave(&mut self, slave: Slave) {
        self.slave_id = slave.into();
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Client for Context<T> {
    fn call<'a>(
        &'a mut self,
        req: Request,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Response>, Error>> + Send + 'a>> {
        Box::pin(self.call(req))
    }
}
