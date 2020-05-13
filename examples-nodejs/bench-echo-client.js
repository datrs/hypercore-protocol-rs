const PORT = 11011
const KEY = Buffer.alloc(32, 0)

const net = require('net')
const Protocol = require('hypercore-protocol')
const pretty = require('pretty-bytes')

const COUNT = 100000
const SIZE = 1000
const data = Buffer.alloc(SIZE, 0)
const conns = 1

for (let i = 0; i < conns; i++) {
  const socket = net.connect(PORT)
  onconnection(socket, i)
}

function onconnection (socket, i) {
  const proto = new Protocol(true)
  socket.pipe(proto).pipe(socket)
  const timer = clock()
  const channel = proto.open(KEY, {
    onopen () {
      channel.data({
        value: data,
        index: 0
      })
    },
    ondata (msg) {
      if (msg.index < COUNT) {
        channel.data({
          value: data,
          index: msg.index + 1
        })
        // if (msg.index % 10000 === 0) {
        //   console.log('done', msg.index)
        // }
      } else {
        const bytes = COUNT * SIZE
        const time = timer()
        console.log('bytes:', pretty(bytes))
        console.log('time:', formatTime(time))
        console.log('throughput', throughput(time, bytes))
        process.exit(0)
      }
    }
  })
}

function clock () {
  const [ss, sn] = process.hrtime()
  return () => {
    const [ds, dn] = process.hrtime([ss, sn])
    const ns = (ds * 1e9) + dn
    return ns
  }
}

function formatTime (ns) {
  const ms = round(ns / 1e6)
  const s = round(ms / 1e3)
  let time
  if (s >= 1) time = s + 's'
  else if (ms >= 0.01) time = ms + 'ms'
  else if (ns) time = ns + 'ns'
  return time
}

function throughput (ns, bytes) {
  const bytespers = pretty(bytes / (ns / 1e9))
  return `${bytespers}/s`
}

function round (num, decimals = 2) {
  return Math.round(num * Math.pow(10, decimals)) / Math.pow(10, decimals)
}
