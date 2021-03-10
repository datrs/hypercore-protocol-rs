const RAM = require('random-access-memory')
const net = require('net')
const pretty = require('pretty-bytes')
const hypercore = require('hypercore')
const { Duplex } = require('streamx')

let n = 5
for (let i = 0; i < n; i++) {
  main(9000 + i).catch(console.error)
}

async function main (port) {
  const feedA = hypercore(RAM)
  await new Promise(resolve => feedA.ready(resolve))
  const feedB = hypercore(RAM, feedA.key)
  await new Promise(resolve => feedB.ready(resolve))

  const server = net.createServer(socket => {
    const proto = feedA.replicate(false, { live: true })
    socket.pipe(proto).pipe(socket)
  })

  await new Promise((resolve, reject) => {
    server.listen(port, err => {
      if (err) return reject(err)
      const socket = net.connect(port, resolve)
      socket.once('error', reject)
      const proto = feedB.replicate(true, { live: true })
      socket.pipe(proto).pipe(socket)
    })
  })
  console.log('connected')


  // const protoA = feedA.replicate(true, { live: true })
  // const protoB = feedB.replicate(false, { live: true })
  // protoA.pipe(protoB).pipe(protoA)



  const extA = streamExtension(feedA, 'ext')
  const extB = streamExtension(feedB, 'ext')

  let limit = 1024 * 1024 * 64
  // let limit = 1024 * 64 * 9
  const timer = clock()

  process.nextTick(() => {
    let len = 0
    extB.on('data', buf => {
      len += buf.length
      // console.log('B recv', buf.length, len)
      extB.write(buf)
    })
  })

  process.nextTick(() => {
    let buf = Buffer.alloc(1024 * 64, 0)
    let len = 0

    next()
    function next () {
      extA.write(buf)
      len += buf.length
      if (len < limit + 1) {
        setImmediate(next)
        // setTimeout(next, 0)
      }
    }
  })

  let printed = false
  await new Promise(resolve => {
    let len = 0
    extA.on('data', buf => {
      len += buf.length
      // console.log('A recv', buf.length, len, limit)
      if (len >= limit) {
        if (!printed) done()
        resolve()
      }
    })
  })

  function done () {
    printed = true
    // console.log('written: ' + pretty(limit))
    const time = timer()
    const formatted = formatTime(time, limit)
    console.log(pretty(limit), formatted)
  }
}

function streamExtension (feed, name) {
  const ext = feed.registerExtension(name, {
    onmessage (message, peer) {
      stream.push(message)
    }
  })
  const stream = new Duplex({
    write (data, cb) {
      ext.broadcast(data)
      cb()
    }
  })
  return stream
}

function clock () {
  const [ss, sn] = process.hrtime()
  return () => {
    const [ds, dn] = process.hrtime([ss, sn])
    const ns = (ds * 1e9) + dn
    return ns
  }
}

function formatTime (ns, bytes) {
  const ms = round(ns / 1e6)
  const s = round(ms / 1e3)
  const bytespers = pretty(bytes / (ns / 1e9))
  let time
  if (s >= 1) time = s + 's'
  else if (ms >= 0.01) time = ms + 'ms'
  else if (ns) time = ns + 'ns'
  return `${time} ${bytespers}/s`
}

function round (num, decimals = 2) {
  return Math.round(num * Math.pow(10, decimals)) / Math.pow(10, decimals)
}
