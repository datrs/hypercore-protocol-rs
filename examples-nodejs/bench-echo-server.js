const PORT = 11011
const KEY = Buffer.alloc(32, 0)

const net = require('net')
const Protocol = require('hypercore-protocol')

net.createServer(onconnection).listen(PORT, () => {
  console.log('listening on localhost:%s', PORT)
})
function onconnection (socket) {
  console.log('new connection from %s:%s', socket.remoteAddress, socket.remotePort)
  socket.on('end', () => {
    console.log('connection closed from %s:%s', socket.remoteAddress, socket.remotePort)
  })
  const proto = new Protocol(false)
  socket.pipe(proto).pipe(socket)
  const channel = proto.open(KEY, {
    ondata (msg) {
      channel.data(msg)
    }
  })
}
