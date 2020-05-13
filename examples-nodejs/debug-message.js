const fs = require('fs')
const { Request } = require('simple-hypercore-protocol/messages.js')

encode('n')
decode('r')

function encode (n) {
  const msg1 = Request.encode({
    index: 127
  })
  const msg2 = Request.encode({
    index: 128
  })
  fs.writeFileSync('msg1' + n, msg1)
  fs.writeFileSync('msg2' + n, msg2)
}
function decode (n) {
  const buf1 = fs.readFileSync('msg1' + n)
  const buf2 = fs.readFileSync('msg2' + n)
  console.log(Request.decode(buf1))
  console.log(Request.decode(buf2))
}
