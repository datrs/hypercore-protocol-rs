const net = require('net')
const pretty = require('pretty-bytes')

const SIZE = 1000
const COUNT = 1000
const PORT = 12345
const ITERS = 100

net.createServer(socket => {
  socket.pipe(socket)
}).listen(PORT, () => {
  const timer = clock()
  let total = 0
  let i = 0
  next()
  function next (time) {
    if (++i <= ITERS) process.nextTick(echobench, i, next)
    else done()
  }
  function done () {
    console.log(`finish ${ITERS} iterations, each ${COUNT} * ${pretty(SIZE)}`)
    console.log(formatTime(timer(), SIZE * COUNT * ITERS))
    process.exit(0)
  }
})

function echobench (j, cb) {
  const timer = clock()
  const socket = net.connect(PORT)
  const data = Buffer.alloc(SIZE, 1)
  // let result = Buffer.alloc(COUNT * SIZE, 0)
  let offset = 0
  let i = 0
  socket.on('data', ondata)
  write()
  function ondata (buf) {
    // result.copy(buf, offset)
    // console.log(j, offset, buf.length, buf.slice(buf.length - 2))
    offset += buf.length
    // console.log(COUNT * SIZE - offset)
    if (offset >= COUNT * SIZE) {
      // console.log('done')
      // socket.removeListener('data', ondata)
      cb(timer())
    }
  }
  function write () {
    socket.write(data)
    // console.log(j, 'written', i * data.length)
    if (++i < COUNT) process.nextTick(write)
    // else console.log(j, 'written', data.length * i)
  }
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
