import { effectScope, ref } from "vue";
import { flushPromises } from "@vue/test-utils";
import { expect, test } from "vitest";
import { useDetection } from "./useDetection";
import type { DetectionApi } from "../lib/ipc";
import type { Settings, SyncPair } from "../lib/contracts";

const a: SyncPair = { id:"11111111-1111-4111-8111-111111111111",name:"A",source_folder:"/a",target_folder:"/b",created_at:"2026-09-19T10:00:00Z",processing_revision:"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",config:{model:"de_core_news_lg",enabled_entities:[],custom_rules:[],include_positions:true} };
const b: SyncPair = {...a,id:"22222222-2222-4222-8222-222222222222",name:"B"};
const settings:Settings={schema_version:1,sync_pairs:[a,b],selected_sync_pair_id:a.id};
const api:DetectionApi={listModels:async()=>[{name:"de_core_news_lg",version:"3.8.0",compatible:true}],refresh:async()=>settings,save:async()=>settings,preview:async()=>[]};

test("late preview results and failures cannot cross a pair switch",async()=>{
  for(const reject of [false,true]) {
    const pair=ref<SyncPair|null>(a),scope=effectScope(); let finish!:(value:never)=>void;
    const detection=scope.run(()=>useDetection(pair,()=>{}, {...api,preview:()=>new Promise((resolve,fail)=>{finish=reject?fail:resolve;})}))!;
    await flushPromises();
    const pending=detection.preview(a.id,a.config,"private A");
    pair.value=b; await flushPromises();
    finish((reject?{code:"engine_timeout",retryable:false}:[]) as never);
    await pending;
    expect(detection.result.value).toBeNull(); expect(detection.error.value).toBeNull();
    expect(detection.busy.value).toBe(false); scope.stop();
  }
});

test("validated saves update settings and failures leave the draft available",async()=>{
  const pair=ref<SyncPair|null>(a),scope=effectScope(); const applied:Settings[]=[];
  const updated={...settings,sync_pairs:[{...a,processing_revision:"cccccccc-cccc-4ccc-8ccc-cccccccccccc",config:{...a.config,include_positions:false}},b]};
  const detection=scope.run(()=>useDetection(pair,value=>applied.push(value), {...api,save:async()=>updated}))!;
  await flushPromises();
  await detection.save(a.id,updated.sync_pairs[0]!.config);
  expect(applied.at(-1)).toEqual(updated); expect(detection.saved.value).toBe(true);
  scope.stop();
  const failureScope=effectScope();
  const failed=failureScope.run(()=>useDetection(pair,value=>applied.push(value),{...api,save:async()=>{throw {code:"invalid_configuration",retryable:false};}}))!;
  await flushPromises(); const count=applied.length;
  await failed.save(a.id,a.config);
  expect(applied).toHaveLength(count); expect(failed.error.value?.code).toBe("invalid_configuration"); expect(failed.busy.value).toBe(false);
  failureScope.stop();
});
