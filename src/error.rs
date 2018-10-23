use maidsafe_utilities::serialisation::SerialisationError;
use mio::timer::TimerError;
use std::io;
use udt_extern::UdtError;
use utp::UtpError;

quick_error! {
    /// Common module specific error
    #[derive(Debug)]
    pub enum SocketError {
        /// IO error
        Io(e: io::Error) {
            description(e.description())
            display("Io error: {}", e)
            cause(e)
            from()
        }
        /// Socket is uninitialised and invalid for any operation
        UninitialisedSocket {
            description("Socket is uninitialised and invalid for any operation")
            display("Socket is uninitialised and invalid for any operation")
        }
        /// Size of a message to send or about to be read is too large
        PayloadSizeProhibitive {
            description("Payload is too large")
        }
        /// Serialisation error
        Serialisation(e: SerialisationError) {
            description(e.description())
            display("Serialisation error: {}", e)
            cause(e)
            from()
        }
        /// Timer error
        Timer(e: TimerError) {
            description(e.description())
            display("Timer error: {}", e)
            cause(e)
            from()
        }
        /// A zero byte socket read - means EOF
        ZeroByteRead {
            description("Read zero bytes from the socket - indicates EOF")
        }
        /// UDT error
        Udt(e: UdtError) {
            description(&e.err_msg)
            display("Udt error: {}", e.err_msg)
            from()
        }
        /// UDP Socket is not connected
        UnconnectedUdpSocket {
            description("UDP Socket is not connected")
        }
        /// No UDT Epoll Loop
        NoUdtEpoll {
            description("No UDT Epoll while registering/deregistering")
        }
        /// UDT read has resulted in negative return value
        UdtNegativeBytesRead(val: i32) {
            description("UDT Read has resulted in a negative result. This is an error value.")
            display("UDT Read has resulted in a negative result. This is an error value: {}", val)
        }
        /// UDT write has resulted in negative return value
        UdtNegativeBytesWrite(val: i32) {
            description("UDT Write has resulted in a negative result. This is an error value.")
            display("UDT Write has resulted in a negative result. This is an error value: {}", val)
        }
        /// uTP related error.
        Utp(e: UtpError) {
            display("uTP failure: {}", e)
            from()
        }
    }
}
