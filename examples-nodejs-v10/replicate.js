const net = require('net')
const Hypercore = require('hypercore');
const RAM = require('random-access-memory')
const { pipeline } = require('stream')
const fs = require('fs')
const p = require('path')
const os = require('os')
const split = require('split2')

const hostname = 'localhost'
let [mode, port, keyOrFilename] = process.argv.slice(2)
if (['client', 'server'].indexOf(mode) === -1 || !port || !keyOrFilename) {
  exit('usage: node index.js [client|server] PORT [KEY|FILENAME]')
}

const KEY_REGEX = /^[d0-9a-f]{64}$/i
let key, filename
if (keyOrFilename.match(KEY_REGEX)) {
  key = keyOrFilename
} else {
  filename = keyOrFilename
}

const hypercore = new Hypercore((_) => new RAM(), key)
hypercore.info().then((_info) => {
  console.log('KEY=' + hypercore.key.toString('hex'))
  console.log()
  if (hypercore.writable && filename) {
    hypercore.append(['hi\n', 'ola\n', 'hello\n', 'mundo\n'])
    // pipeline(
    //   fs.createReadStream(filename),
    //   split(),
    //   feed.createWriteStream(),
    //   err => {
    //     if (err) console.error('error importing file', err)
    //     else console.error('import done, new len %o, bytes %o', feed.length, feed.byteLength)
    //   }
    // )
  }
})

const opts = {
  hypercore, filename, mode, port, hostname
}

start(opts)

function start (opts) {
  const { port, hostname, mode, hypercore, filename } = opts
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
  const { socket, isInitiator, feed } = opts
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

  // const proto = new Protocol(isInitiator, { noise: true, encrypted: false })
  feed.ready(() => {
    let mode = feed.writable ? 'write' : 'read'
    const proto = feed.replicate(isInitiator, { encrypted: true, live: true })

    console.error('init protocol')
    console.error('key', feed.key.toString('hex'))

    proto.pipe(socket).pipe(proto)

    proto.on('error', err => {
      console.error('protocol error', err)
      socket.destroy()
    })

    if (mode === 'write') {
      // feed.append(feed.length)
      // feed.append('hello')
      // setTimeout(() => feed.append('world'), 500)

      // const filepath = p.join(os.homedir(), 'Musik', 'foo.mp3')
      // const rs = fs.createReadStream(filepath)
      // rs.pipe(feed.createWriteStream())
    }
    if (mode === 'read') {
      feed.createReadStream({ live: true }).pipe(process.stdout)
    }

    // setTimeout(() => proto.destroy(), 1000)
  })
}

function exit (msg) {
  console.error(msg)
  process.exit(1)
}
