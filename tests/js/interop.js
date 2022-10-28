const Hypercore = require('hypercore');
const net = require('net');
const fs = require('fs').promises;

// Static test key pair obtained with:
//
//   const crypto = require('hypercore-crypto');
//   const keyPair = crypto.keyPair();
//   console.log("public key", keyPair.publicKey.toString('hex').match(/../g).join(' '));
//   console.log("secret key", keyPair.secretKey.toString('hex').match(/../g).join(' '));
const testKeyPair = {
    publicKey: Buffer.from([
        0x97, 0x60, 0x6c, 0xaa, 0xd2, 0xb0, 0x8c, 0x1d, 0x5f, 0xe1, 0x64, 0x2e, 0xee, 0xa5, 0x62, 0xcb,
        0x91, 0xd6, 0x55, 0xe2, 0x00, 0xc8, 0xd4, 0x3a, 0x32, 0x09, 0x1d, 0x06, 0x4a, 0x33, 0x1e, 0xe3]),
    secretKey: Buffer.from([
        0x27, 0xe6, 0x74, 0x25, 0xc1, 0xff, 0xd1, 0xd9, 0xee, 0x62, 0x5c, 0x96, 0x2b, 0x57, 0x13, 0xc3,
        0x51, 0x0b, 0x71, 0x14, 0x15, 0xf3, 0x31, 0xf6, 0xfa, 0x9e, 0xf2, 0xbf, 0x23, 0x5f, 0x2f, 0xfe,
        0x97, 0x60, 0x6c, 0xaa, 0xd2, 0xb0, 0x8c, 0x1d, 0x5f, 0xe1, 0x64, 0x2e, 0xee, 0xa5, 0x62, 0xcb,
        0x91, 0xd6, 0x55, 0xe2, 0x00, 0xc8, 0xd4, 0x3a, 0x32, 0x09, 0x1d, 0x06, 0x4a, 0x33, 0x1e, 0xe3]),
}
const hostname = 'localhost'

if (process.argv.length !== 9 || process.argv[7].length != 1) {
    console.error("Usage: node interop.js [server/client] [writer/reader] [port] [count of items to replicate] [size in bytes of items] [character to repeat in item data] [test set]")
    process.exit(1);
}

const isWriter = process.argv[3] === 'writer';
const port = parseInt(process.argv[4]);
const itemCount = parseInt(process.argv[5]);
const itemSize = parseInt(process.argv[6]);
const itemChar = process.argv[7];
const testSet = process.argv[8];
const resultFile = `work/${testSet}/result.txt`;

if (process.argv[2] === 'server') {
    runServer(isWriter, itemCount, itemSize, itemChar, testSet).then(_ => {
        console.log("NODE: Server created");
    });
} else if (process.argv[2] === 'client') {
    runClient(isWriter, itemCount, itemSize, itemChar, testSet).then(_ => {
        console.log("NODE: client run");
    });
} else {
    console.error(`Invalid mode {}, only server/client supported`, process.argv[2]);
    process.exit(2);
}

async function runServer(isWriter, itemCount, itemSize, itemChar, testSet) {
    const isInitiator = false;
    const hypercore = isWriter ? await createWriteHypercore(itemCount, itemSize, itemChar, testSet) : await createReadHypercore(testSet);
    const server = net.createServer(async socket => onconnection({ isInitiator, hypercore, socket, itemCount }))
    try {
      server.listen(port, hostname, async () => { const { address, port } = server.address()
        console.error(`NODE: server listening on ${address}:${port}`)
      });
    } catch (error) {
      console.error(`NODE: ${isInitiator} server listen got error`, error);
    }
}

async function runClient(isWriter, itemCount, itemSize, itemChar, testSet) {
    const isInitiator = true;
    const hypercore = isWriter ? await createWriteHypercore(itemCount, itemSize, itemChar, testSet) : await createReadHypercore(testSet);
    const socket = await net.connect(port, hostname);
    await onconnection({ isInitiator, hypercore, socket, itemCount });
}

class Mutex {
  constructor () {
    this.locked = false
    this.destroyed = false

    this._destroying = null
    this._destroyError = null
    this._queue = []
    this._enqueue = (resolve, reject) => this._queue.push([resolve, reject])
  }

  lock () {
    if (this.destroyed) return Promise.reject(this._destroyError)
    if (this.locked) return new Promise(this._enqueue)
    this.locked = true
    return Promise.resolve()
  }

  unlock () {
    if (!this._queue.length) {
      this.locked = false
      return
    }
    this._queue.shift()[0]()
  }

  destroy (err) {
    if (!this._destroying) this._destroying = this.locked ? this.lock().catch(() => {}) : Promise.resolve()

    this.destroyed = true
    this._destroyError = err || new Error('Mutex has been destroyed')

    if (err) {
      while (this._queue.length) this._queue.shift()[1](err)
    }

    return this._destroying
  }
}

let mutex = new Mutex()
async function onconnection (opts) {
  const { isInitiator, hypercore, socket, itemCount } = opts
  const { remoteAddress, remotePort } = socket
  if (!isInitiator) {
    console.error(`NODE: new connection from ${remoteAddress}:${remotePort}`)
  }
  socket.on('close', () => {
    if (!isInitiator) {
      console.error(`NODE: connection closed from ${remoteAddress}:${remotePort}`)
    } else {
      console.error('NODE: connection closed from server')
    }
  })

  hypercore.on('append', async _ => {
      await mutex.lock()
      console.error(`NODE: ${isInitiator} got append, new length ${hypercore.length} and byte length ${hypercore.byteLength}, count match=${hypercore.length === itemCount}`)
      if (hypercore.length === itemCount) {
          let fileContent = "";
          for (let i = 0; i < hypercore.length; i++) {
              console.error(`${isInitiator} Getting value for index ${i}`);
              let value = await hypercore.get(i);
              fileContent += `${i} ${value}\n`;
          }
          try {
              console.error(`NODE: ${isInitiator} Writing file`);
              await fs.writeFile(resultFile, fileContent);
          } catch (error) {
              console.error(`NODE: ${isInitiator} got error`, error);
              process.exit(3);
          }

          console.error(`NODE: ${isInitiator} Wrote content exiting`);
          process.exit(0);
      }
      mutex.unlock()
  })
  socket.pipe(hypercore.replicate(isInitiator)).pipe(socket)
}

async function createWriteHypercore(itemCount, itemSize, itemChar, testSet){
    const core = new Hypercore(`work/${testSet}/writer`, testKeyPair.publicKey, {keyPair: testKeyPair});
    let data = Buffer.alloc(itemSize, itemChar);
    for (let i=0; i<itemCount; i++) {
        await core.append(data);
    }
    return core;
}

async function createReadHypercore(testSet) {
    return new Hypercore(`work/${testSet}/reader`, testKeyPair.publicKey);
}
