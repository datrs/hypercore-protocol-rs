const net = require('net')
const Protocol = require('hypercore-protocol')
const hypercore = require('hypercore')
const ram = require('random-access-memory')
const { pipeline } = require('stream')
const fs = require('fs')
const p = require('path')
const os = require('os')

const hostname = 'localhost'
let [mode, port, keyOrFilename] = process.argv.slice(2)
if (['client', 'server'].indexOf(mode) === -1 || !port) {
  exit('usage: node index.js [client|server] PORT [KEY|FILENAME]')
}

const KEY_REGEX = /^[\dabcdef]{64}$/i
let key, filename
if (keyOrFilename.match(KEY_REGEX)) {
  key = keyOrFilename
} else {
  filename = keyOrFilename
}

const feed = hypercore(ram, key)
feed.ready(() => {
  console.log('key', feed.key.toString('hex'))
})

const opts = {
  feed, filename, mode, port, hostname
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

    if (feed.writable && filename) {
      pipeline(
        fs.createReadStream(filename),
        feed.createWriteStream(),
        err => {
          if (err) console.error('error importin file', err)
          else console.error('import done, new len', feed.length)
        }
      )
    }

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
  })
}

function exit (msg) {
  console.error(msg)
  process.exit(1)
}
