import { writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnServer } from '../helpers/spawn-server'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const INFO_PATH = path.resolve(__dirname, '.server-info.json')

export default async function globalSetup(): Promise<void> {
  const server = await spawnServer()
  writeFileSync(INFO_PATH, JSON.stringify({ port: server.port, pid: server.process.pid, dbDir: server.dbDir }))
}
