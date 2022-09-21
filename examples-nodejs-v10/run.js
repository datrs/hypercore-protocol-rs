const { spawn } = require('child_process')
const p = require('path')
const chalk = require('chalk')
const split = require('split2')

const PORT = 8000

const EXAMPLE_NODE = p.join(__dirname, 'replicate.js')
const EXAMPLE_RUST = process.argv[2]
if (!EXAMPLE_RUST) {
  usage()
}
const MODE = process.argv[3] || 'nodeServer'

function startNode (mode, key, color, name) {
  const args = [EXAMPLE_NODE, mode, PORT]
  if (key) args.push(key)
  const node = start({
    bin: 'node',
    args,
    name: name || 'node',
    color: color || 'red',
    env: {
      ...process.env
    }
  })
  return node
}

function startRust (mode, key, color, name) {
  const args = ['run', '--example', EXAMPLE_RUST, '--', mode, PORT]
  if (key) args.push(key)
  const rust = start({
    bin: 'cargo',
    args,
    name: name || 'rust',
    color: color || 'blue',
    env: {
      ...process.env,
      RUST_LOG_STYLE: 'always'
    }
  })
  return rust
}

let client, server
if (MODE === 'nodeServer') {
  server = startNode
  client = startRust
} else if (MODE === 'rustServer') {
  server = startRust
  client = startNode
} else if (MODE === 'node') {
  server = startNode
  client = startNode
} else if (MODE === 'rust') {
  server = startRust
  client = startRust
}

const procs = []
const proc = server('server', null, 'red')
procs.push(proc)
proc.once('stdout-line', line => {
  const [, key] = line.split('=')
  client('client', key, 'blue')
})

process.on('SIGINT', onclose)

function onclose () {
  setTimeout(() => {
    procs.forEach(proc => proc.kill())
    process.exit()
  }, 100)
}

function start ({ bin, args, name, color, env = {} }) {
  console.log(chalk[color].bold(`[${name}] spawn: `) + chalk[color](`${bin} ${args.join(' ')}`))
  const proc = spawn(bin, args, {
    env: { ...process.env, ...env }
  })
  proc.on('exit', onclose)
  proc.stderr.pipe(split()).on('data', line => {
    proc.emit('stderr-line', line)
    console.error(chalk[color]('[' + name + ']') + ' ' + line)
  })
  proc.stdout.pipe(split()).on('data', line => {
    proc.emit('stdout-line', line)
    console.log(chalk.bold[color]('[' + name + ']') + ' ' + line)
  })
  return proc
}

function usage () {
  console.error('USAGE: node run.js [basic_v10|hypercore_v10]')
  process.exit(1)
}
