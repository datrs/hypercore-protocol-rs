const net = require('net')
const Hypercore = require('hypercore');
const RAM = require('random-access-memory')

const hostname = 'localhost'
let [mode, port, key] = process.argv.slice(2)
if (['client', 'server'].indexOf(mode) === -1 || !port) {
  exit('usage: node replicate.js [client|server] PORT (KEY)')
}
const hypercore = new Hypercore((_) => new RAM(), key)
hypercore.info().then((_info) => {
  console.log('KEY=' + hypercore.key.toString('hex'))
  console.log()
  if (hypercore.writable && !key) {
    hypercore.append(['hi\n', 'ola\n', 'hello\n', 'mundo\n'])
  }
})

const opts = {
  hypercore, mode, port, hostname
}

start(opts)

function start (opts) {
  const { port, hostname, mode } = opts
  const isInitiator = mode === 'client'
  opts.isInitiator = isInitiator

  if (mode === 'client') {
    const socket = net.connect(port, hostname)
    onconnection({ ...opts, socket })
  } else {
    const server = net.createServer(socket => onconnection({ ...opts, socket }))
    server.listen(port, hostname, () => {
      const { address, port } = server.address()
      console.error(`server listening on ${address}:${port}`)
    })
  }
}

function onconnection (opts) {
  const { socket, isInitiator, mode, hypercore } = opts
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

  hypercore.on('append', _ => {
      console.log(`${mode} got append, new length ${hypercore.length} and byte length ${hypercore.byteLength}, replaying:`)
      for (let i = 0; i < hypercore.length; i++) {
          hypercore.get(i).then(value => {
             console.log(`${i}: ${value}`);
          });
      }
  })
  socket.pipe(hypercore.replicate(isInitiator)).pipe(socket)
}

function exit (msg) {
  console.error(msg)
  process.exit(1)
}
