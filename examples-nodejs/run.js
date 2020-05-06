const { spawn } = require('child_process')
const p = require('path')
const chalk = require('chalk')
const split = require('split2')

const PORT = 8000
const FILE = p.join(__dirname, '..', 'README.md')

const EXAMPLE_NODE = p.join(__dirname, 'replicate.js')
const EXAMPLE_RUST = process.argv[2]
if (!EXAMPLE_RUST) {
  usage()
}
const SERVER = process.argv[3] || 'node'

function startNode (mode, key, color, name) {
  const args = [EXAMPLE_NODE, mode, PORT]
  if (key) args.push(key)
  else args.push(FILE)
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
if (SERVER === 'node') {
  server = startNode
  client = startRust
} else {
  server = startRust
  client = startNode
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
  console.error('USAGE: node run.js [basic|hypercore]')
  process.exit(1)
}
