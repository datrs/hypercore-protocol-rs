const net = require('net')
const Protocol = require('hypercore-protocol')

const KEY = Buffer.from('01234567890123456789012345678901')
const hostname = 'localhost'
let [mode, port] = process.argv.slice(2)
if (['client', 'server'].indexOf(mode) === -1 || !port) {
  exit('usage: node index.js [client|server] PORT')
}

start({ port, hostname, mode })

function start ({ port, hostname, mode }) {
  const isInitiator = mode === 'client'
  if (mode === 'client') {
    const socket = net.connect(port, hostname)
    onconnection({ socket, isInitiator })
  } else {
    const server = net.createServer(socket => onconnection({ socket, isInitiator }))
    server.listen(port, hostname, () => {
      const { address, port } = server.address()
      console.log(`server listening on ${address}:${port}`)
    })
  }
}

function onconnection (opts) {
  const { socket, isInitiator } = opts
  const { remoteAddress, remotePort } = socket
  if (!isInitiator) {
    console.log(`new connection from ${remoteAddress}:${remotePort}`)
  }
  socket.on('close', () => {
    if (!isInitiator) {
      console.log(`connection closed from ${remoteAddress}:${remotePort}`)
    } else {
      console.log('connection closed from server')
    }
  })

  const proto = new Protocol(isInitiator, { noise: true, encryption: false })

  console.log('init protocol')
  console.log('local public key: ', proto.publicKey)

  proto.pipe(socket).pipe(proto)

  proto.on('error', err => {
    console.log('protocol error', err)
    socket.destroy()
  })
  proto.on('handshake', () => {
    console.log('handshake finished')
    console.log('remote public key:', proto.remotePublicKey)
    console.log('noise handshake nonces', {
      local: proto.state._payload.nonce,
      remote: proto.state.remotePayload.nonce
    })
    console.log('noise handshake split lengths:', { rx: proto.state._split.rx.length, tx: proto.state._split.tx.length })
    console.log('noise handshake split:', proto.state._split)
    console.log('now open channel')
    proto.open(KEY, {
      onopen () {
        console.log('channel opened!')
      }
    })
  })
}

function exit (msg) {
  console.error(msg)
  process.exit(1)
}
