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
      console.error(`server listening on ${address}:${port}`)
    })
  }
}

function onconnection (opts) {
  const { socket, isInitiator } = opts
  const { remoteAddress, remotePort } = socket
  if (!isInitiator) {
    console.error(`new connection from ${remoteAddress}:${remotePort}`)
  }
  socket.on('close', () => {
    if (!isInitiator) {
      console.error(`connection closed from ${remoteAddress}:${remotePort}`)
    } else {
      console.error('connection closed from server')
    }
  })

  const proto = new Protocol(isInitiator, { noise: true, encrypted: false })

  console.error('init protocol')
  console.error('local public key: ', proto.publicKey)

  proto.pipe(socket).pipe(proto)

  proto.on('error', err => {
    console.error('protocol error', err)
    socket.destroy()
  })
  proto.on('handshake', () => {
    console.error('handshake finished')
    console.error('remote public key:', proto.remotePublicKey)
    console.error('noise handshake nonces', {
      local: proto.state._payload.nonce,
      remote: proto.state.remotePayload.nonce
    })
    console.error('noise handshake split lengths:', { rx: proto.state._split.rx.length, tx: proto.state._split.tx.length })
    console.error('noise handshake split:', proto.state._split)
    setTimeout(() => {
      console.error('now open channel')
      proto.open(KEY, {
        onopen () {
          console.error('channel opened!')
        }
      })
    }, 0)
  })
}

function exit (msg) {
  console.error(msg)
  process.exit(1)
}
