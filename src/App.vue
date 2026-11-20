<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { listen, type Event } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-dialog'
import { load } from '@tauri-apps/plugin-store'
import { cloneDeep, isEqual } from 'lodash-es'
import { onBeforeMount, onBeforeUnmount, onMounted, ref } from 'vue'
import { readText } from '@tauri-apps/plugin-clipboard-manager'

import { stringFormatter } from './composables/helper'
import {
    createTaskId,
    findTaskById,
    finishedProgress,
    hasPendingTasks,
    nextPendingTask,
    taskHasWork,
} from './composables/taskQueue'
import { useStore } from './store/index.ts'

import AlertMsg from './components/AlertMsg.vue'
import EditConfig from './components/EditConfig.vue'
import EditPresets from './components/EditPresets.vue'
import EditPublisher from './components/EditPublisher.vue'
import EditTemplate from './components/EditTemplate.vue'
import HeaderMenu from './components/HeaderMenu.vue'
import LogWindow from './components/LogWindow.vue'
import MediaTable from './components/MediaTable.vue'
import AddUrl from './components/AddUrl.vue'

const { folderPath, filename, removeExtension, Logger } = stringFormatter()

const store = useStore()

const defaultTemplate: Template = {
    intro: '',
    outro: '',
    lower_thirds: [],
}
const currentTask = ref<Task | null>(null)

const targetFolder = ref<string | null>(null)
const targetSubfolder = ref(false)
const noProgressValues = ref(false)
const showTemplateEditor = ref(false)
const showPublisherEditor = ref(false)
const showUrlDialog = ref(false)
const pendingUrl = ref('')

const log = new Logger()

function errorMessage(error: unknown): string {
    if (error instanceof Error) return error.message
    if (typeof error === 'string') return error
    try {
        return JSON.stringify(error)
    } catch {
        return String(error)
    }
}

async function pasteUrl(event: KeyboardEvent) {
    if (event.defaultPrevented || !(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== 'v') return
    const element = event.target as HTMLElement | null
    if (element?.matches('input, textarea, [contenteditable="true"]')) return

    const url = (await readText()).match(/https?:\/\/[^\s"']+/)?.[0]
    if (url) {
        event.preventDefault()
        showAddUrl(url)
    }
}

onMounted(() => window.addEventListener('keydown', pasteUrl))
onBeforeUnmount(() => window.removeEventListener('keydown', pasteUrl))

onBeforeMount(async () => {
    await invoke('load_config').catch((e) => {
        store.msgAlert('error', e, 5)
        log.error(e)
    })

    const config = await load('config.json', { autoSave: false, defaults: {} })
    store.showTranscript = (await config.get('transcript_cmd')) ? true : false
    store.transcriptLanguages = (await config.get('transcript_lang')) ?? []
    store.publishPreset = (await config.get('publish_preset')) ?? ''

    if (store.transcriptLanguages.length === 0) {
        store.transcriptLanguages = [
            { name: 'None', code: 'none' },
            { name: 'Auto', code: 'auto' },
            { name: 'Multilingual', code: 'ml' },
            { name: 'German', code: 'de' },
            { name: 'English', code: 'en' },
            { name: 'Spanish', code: 'es' },
        ]

        await config.set('transcript_lang', store.transcriptLanguages)
        await config.save()
    }

    await invoke<Preset[]>('presets_get')
        .then((prs: Preset[]) => {
            if (store.presets.length === 0) {
                for (const preset of prs) {
                    store.presets.push(preset)
                }
            }
        })
        .catch((e) => {
            store.msgAlert('error', e, 5)
            log.error(e)
        })
})

listen<Task>('task-active', (event: Event<Task>) => {
    const task = findTaskById(store.taskList, event.payload.id)
    if (task) {
        task.active = true
        store.processPath = filename(task.path)
    }
})

listen<Task>('task-finish', (event: Event<Task>) => {
    const task = findTaskById(store.taskList, event.payload.id)
    if (!task || !store.jobInProcess) return

    task.active = false
    task.finished = true
    store.progressAll = finishedProgress(store.taskList)

    if (nextPendingTask(store.taskList)) {
        void taskSendNext()
    } else if (!hasPendingTasks(store.taskList)) {
        store.jobInProcess = false
        store.jobsDone = true
    }
})

listen<String>('lufs-progress', async (event: Event<FFmpegProgress>) => {
    noProgressValues.value = false
    store.progressCurrent = event.payload.elapsed_pct
    store.processMsg = `<strong>Analyze (${event.payload.title} ${event.payload.speed} Speed): </strong>`
})

listen<String>('preset-start', async (event: Event<Preset>) => {
    noProgressValues.value = false
    for (const preset of currentTask.value?.presets) {
        if (preset.title === event.payload.title) {
            preset.output_path = event.payload.output_path
        }
    }
})

listen<String>('preset-progress', async (event: Event<FFmpegProgress>) => {
    const progress = event.payload.fps ? `${Math.round(event.payload.fps)} FPS` : `${event.payload.speed} Speed`
    store.progressCurrent = event.payload.elapsed_pct
    store.processMsg = `<strong>Encode (${event.payload.title} ${progress}): </strong>`
})

listen<String>('preset-finish', async (event: Event<Preset>) => {
    store.progressCurrent = 100
    store.processMsg = `<strong>Done (${event.payload.title}): </strong>`

    const index = currentTask.value.presets.findIndex((item: Task) => item.name === event.payload.name)
    currentTask.value.presets.splice(index, 1)
})

listen<string>('transcript-start', async () => {
    noProgressValues.value = true
    store.processMsg = `<strong>Transcript: </strong>`
})

listen<string>('transcript-progress', async (event: Event<string>) => {
    noProgressValues.value = false
    store.progressCurrent = parseFloat(event.payload)
    store.processMsg = `<strong>Transcript: </strong>`
})

listen<string>('transcript-finish', async (event: Event<string>) => {
    store.progressCurrent = 100
    store.processMsg = `<strong>Transcript (${event.payload}) done: </strong>`
})

listen<string>('download-start', (event: Event<string>) => {
    store.downloadInProgress = true
    noProgressValues.value = false
    store.progressCurrent = 0
    store.processPath = event.payload
    store.processMsg = '<strong>Download: </strong>'
})

listen<number>('download-progress', (event: Event<number>) => {
    store.progressCurrent = Math.round(event.payload)
})

listen<string>('download-finish', (event: Event<string>) => {
    store.progressCurrent = 100
    store.processPath = filename(event.payload)
    store.processMsg = '<strong>Download complete: </strong>'
})

listen<string>('logging', (event: Event<string>) => {
    store.logContent.push(event.payload)

    if (event.payload.includes('[ERROR]')) {
        store.msgAlert('error', event.payload.replace('[ERROR]', ''), 5)
    }

    while (store.logContent.length > 5000) {
        store.logContent.shift()
    }
})

async function getDir() {
    const path = store.taskList[store.taskList.length - 1]?.path
    let options = {
        multiple: false,
        directory: true,
    } as any

    if (path) {
        options.defaultPath = folderPath(path)
    }

    targetFolder.value = (await open(options)) as string | null
}

async function taskSendNext() {
    const task = nextPendingTask(store.taskList)
    if (!task) {
        if (!hasPendingTasks(store.taskList)) {
            store.jobInProcess = false
            store.jobsDone = true
        }
        return
    }

    if (!taskHasWork(task)) {
        task.finished = true
        store.progressAll = finishedProgress(store.taskList)
        store.msgAlert('warning', `Skipped ${filename(task.path)}: no transcription, preset or publisher selected.`, 5)
        await taskSendNext()
        return
    }

    store.jobInProcess = true
    store.jobsDone = false
    task.active = true
    task.target = targetFolder.value
    task.target_subfolder = targetSubfolder.value

    if (task.template && !task.template.intro && !task.template.outro && task.template.lower_thirds.length === 0) {
        task.template = null
    }

    currentTask.value = task
    if (task.url) {
        try {
            const path = await invoke<string>('download_url', { url: task.url, target: targetFolder.value })
            task.path = path
            task.url = null
            const downloadedTask = await invoke<Task>('file_drop', { task })
            Object.assign(task, downloadedTask)
        } catch (e) {
            task.active = false
            store.jobInProcess = false
            console.error('yt-dlp download failed:', e)
            const message = errorMessage(e)
            store.msgAlert('error', message, 5)
            log.error(message)
            return
        } finally {
            store.downloadInProgress = false
        }

        if (!taskHasWork(task)) {
            task.active = false
            task.finished = true
            store.progressAll = finishedProgress(store.taskList)
            await taskSendNext()
            return
        }
    }

    try {
        await invoke<Task>('task_send', { task })
    } catch (e) {
        task.active = false
        store.jobInProcess = false
        const message = errorMessage(e)
        console.error('Could not enqueue task:', e)
        store.msgAlert('error', message, 5)
        log.error(message)
    }
}

async function jobRun() {
    if (store.jobInProcess) {
        store.jobInProcess = false

        await invoke<Task>('task_cancel', { task: currentTask.value })
            .then(() => {
                currentTask.value.active = false
                currentTask.value.finished = false
            })
            .catch((e) => {
                store.msgAlert('error', e, 5)
                log.error(e)
            })
    } else {
        // start encoding job
        store.jobsDone = false
        await invoke('task_start').catch((e) => {
            store.msgAlert('error', e, 5)
            log.error(e)
        })

        await taskSendNext()
    }
}

function editTemplate(task: Task) {
    currentTask.value = task
    showTemplateEditor.value = true

    store.currentTemplate = cloneDeep(task.template)
}

async function saveTemplate(update: boolean) {
    if (!update) {
        showTemplateEditor.value = false
    } else if (isEqual(defaultTemplate, store.currentTemplate)) {
        showTemplateEditor.value = false
    } else {
        const path = removeExtension(currentTask.value.path) + '.json'

        await invoke<Task>('template_save', { template: store.currentTemplate, path })
            .then(() => {
                store.msgAlert('success', `Save template ${filename(path)} success.`, 3)

                currentTask.value.template = cloneDeep(store.currentTemplate)
                store.currentTemplate.value = cloneDeep(defaultTemplate)
                showTemplateEditor.value = false
            })
            .catch((e) => {
                store.msgAlert('error', e, 5)
                log.error(e)
            })
    }
}

function editPublisher(task: Task) {
    currentTask.value = task
    showPublisherEditor.value = true

    // store.currentPublisher = cloneDeep(task.template)
}

function savePublisher(_save: boolean) {
    showPublisherEditor.value = false

    // store.currentPublisher = cloneDeep(task.template)
}

async function addFiles() {
    const path = store.taskList[store.taskList.length - 1]?.path
    let options = {
        multiple: true,
        directory: false,
        filters: [
            {
                name: 'File Types',
                extensions: store.ALLOWED_EXTENSIONS,
            },
        ],
    } as any

    if (path) {
        options.defaultPath = folderPath(path)
    }

    let files = (await open(options)) || []

    for (const file of files) {
        const task = cloneDeep(store.defaultTask)
        task.id = createTaskId()

        if (store.taskList.some((task: Task) => task.path === file)) {
            store.msgAlert('warning', `File: <strong>${filename(file)}</strong> already in list!`, 5)
            continue
        }

        task.path = file

        await invoke<Task>('file_drop', { task })
            .then((task: Task) => {
                if (!task.template) {
                    task.template = cloneDeep(store.defaultTemplate)
                }
                store.taskList.push(task)
            })
            .catch((e) => {
                store.msgAlert('error', e, 5)
                log.error(e)
            })
    }
}

function showAddUrl(url = '') {
    pendingUrl.value = url
    showUrlDialog.value = true
}

async function addUrl(url: string) {
    showUrlDialog.value = false
    if (!url) return

    if (store.taskList.some((item: Task) => item.url === url)) {
        store.msgAlert('warning', 'URL is already in the queue.', 5)
        return
    }

    const task = cloneDeep(store.defaultTask)
    task.id = createTaskId()
    task.path = url
    task.url = url
    task.template = cloneDeep(store.defaultTemplate)
    store.taskList.push(task)

    try {
        const version = await invoke<string>('yt_dlp_version')
        console.debug(`yt-dlp ${version} found`)
    } catch (e) {
        const message = errorMessage(e)
        console.warn('yt-dlp availability check failed:', e)
        store.msgAlert('warning', message, 8)
        log.warn(message)
    }
}
</script>

<template>
    <div class="flex flex-col h-screen justify-between select-none cursor-default overflow-hidden">
        <HeaderMenu :logger="log" :add-files="addFiles" :add-url="showAddUrl" />
        <main class="mb-auto bg-base-300 w-full h-full overflow-x-hidden overflow-y-auto">
            <div class="relative bg-base-200 h-full">
                <MediaTable :logger="log" :editTemplate="editTemplate" :editPublisher="editPublisher" :add-files="addFiles" :add-url="showAddUrl" />
                <LogWindow v-if="store.openLog" />
                <EditConfig v-if="store.showConfig" :logger="log" />
                <EditPresets v-if="store.showPresets" :logger="log" />
            </div>
        </main>

        <footer class="relative z-30 h-25.75">
            <div v-if="!store.openLog" class="absolute w-full flex justify-center -top-3">
                <button
                    class="w-20 h-3 min-h-3 btn bg-base-100 rounded-none border-b-0 hover:border-base-content/30 rounded-t-box border-t border-base-content/30 hover:text-base-content/50 pt-1"
                    title="Open Logging"
                    @click="store.openLog = true"
                >
                    <i class="bi-chevron-compact-up" />
                </button>
            </div>
            <div class="flex bg-base-100 border-t border-base-content/30">
                <div class="flex justify-center m-auto item-center w-2/5">
                    <div class="container px-4 flex flex-col gap-0 mb-1">
                        <div class="flex items-center gap-4">
                            <div class="font-semibold w-15">Current:</div>
                            <div class="relative grow flex items-center">
                                <progress
                                    v-if="noProgressValues"
                                    class="progress progress-accent bg-base-content/20 rounded-xs [&::-webkit-progress-value]:rounded-xs h-4"
                                />
                                <template v-else>
                                    <progress
                                        class="progress progress-accent bg-base-content/20 rounded-xs [&::-webkit-progress-value]:rounded-xs h-4"
                                        :value="store.progressCurrent"
                                        max="100"
                                    />
                                    <div class="absolute w-full font-semibold text-center text-xs">
                                        {{ store.progressCurrent }}%
                                    </div>
                                </template>
                            </div>
                        </div>
                        <div class="flex items-center gap-4 mt-2">
                            <div class="font-semibold w-15">Over All:</div>
                            <div class="relative grow flex items-center">
                                <progress
                                    class="progress progress-accent bg-base-content/20 rounded-xs [&::-webkit-progress-value]:rounded-xs h-4"
                                    :value="store.progressAll"
                                    max="100"
                                />
                                <div class="absolute w-full font-semibold text-center text-xs">
                                    {{ store.progressAll }}%
                                </div>
                            </div>
                        </div>
                    </div>
                </div>
                <div class="flex justify-center m-auto item-center w-3/5">
                    <div class="container flex">
                        <div class="p-4 flex flex-col gap-1 w-[calc(100%-102px)]">
                            <div class="flex items-center">
                                <div
                                    class="grow font-semibold truncate pr-2 h-6.25"
                                    v-html="store.processMsg + store.processPath"
                                />

                                <label class="label cursor-pointer pr-0 pt-0 pb-1.25" :disabled="store.jobInProcess">
                                    <span class="label-text mr-2">Subfolder</span>
                                    <input
                                        type="checkbox"
                                        v-model="targetSubfolder"
                                        class="checkbox checkbox-sm checked:shadow-none rounded-xs"
                                    />
                                </label>
                            </div>
                            <div class="flex items-end">
                                <label class="cursor-pointer join w-full">
                                    <input
                                        v-model="targetFolder"
                                        type="text"
                                        class="input input-sm input-bordered focus:border-base-content/30 focus:outline-base-content/30 rounded-xs join-item w-full"
                                        :class="{ 'disabled:input-bordered': store.jobInProcess }"
                                        :disabled="store.jobInProcess || store.downloadInProgress"
                                    />
                                    <button
                                        class="btn btn-sm border-base-content/30 hover:border-base-content/40 rounded-xs join-item"
                                        @click="getDir()"
                                        :disabled="store.jobInProcess || store.downloadInProgress"
                                    >
                                        Save As
                                    </button>
                                </label>
                            </div>
                        </div>
                        <div class="flex items-end pb-4 pr-4">
                            <button
                                class="btn btn-lg border-base-content/30 hover:border-base-content/40 rounded-xs w-20 h-16"
                                @click="jobRun()"
                            >
                                {{ store.jobInProcess ? 'Cancel' : 'Run' }}
                            </button>
                        </div>
                    </div>
                </div>
            </div>
        </footer>
        <AlertMsg v-if="!store.openLog" />
        <EditTemplate :show="showTemplateEditor" :currentTask="currentTask" :saveTemplate="saveTemplate" />
        <EditPublisher
            v-if="showPublisherEditor"
            :show="showPublisherEditor"
            :logger="log"
            :currentTask="currentTask"
            :savePublisher="savePublisher"
        />
        <AddUrl :show="showUrlDialog" :initial-url="pendingUrl" :add-url="addUrl" />
    </div>
</template>
