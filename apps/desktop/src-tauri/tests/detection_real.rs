use redactio_lib::{
    domain::{
        detection,
        mapping::DocumentState,
        settings::{
            load_settings, save_settings, CustomRule, EntityType, ProcessingConfig, Settings,
        },
        sync::{RunController, RunOutcome},
    },
    sidecar::Sidecar,
};
use std::{fs, process::Command};
use uuid::Uuid;

fn rule(pattern: &str, model: &str) -> ProcessingConfig {
    ProcessingConfig {
        model: model.into(),
        enabled_entities: vec![],
        custom_rules: vec![CustomRule::Regex {
            id: Uuid::new_v4(),
            entity_type: EntityType::Custom,
            enabled: true,
            pattern: pattern.into(),
        }],
        include_positions: true,
    }
}

#[test]
#[ignore = "requires reviewed Python and bundled lg/sm packages"]
fn real_two_pair_configuration_model_switch_and_catastrophic_preview_timeout() {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let root=tempfile::tempdir().unwrap(); let config=root.path().join("config"); fs::create_dir(&config).unwrap();
        let python=std::env::var_os("REDACTIO_TEST_PYTHON").unwrap();
        let model_root=std::env::var_os("REDACTIO_MODEL_DIR").unwrap();
        let sidecar=Sidecar::new(python.clone().into(),vec!["-m".into(),"redactio_sidecar".into()],model_root.into());
        let mut settings=Settings::default();
        for name in ["A","B"] {
            let source=root.path().join(format!("{name}-in"));let target=root.path().join(format!("{name}-out"));
            fs::create_dir(&source).unwrap();fs::create_dir(&target).unwrap();settings.add(name,&source,&target).unwrap();
            assert!(Command::new(&python).args(["-c","from docx import Document; import sys; d=Document(); d.add_paragraph('CanaryAlpha CanaryBeta'); d.save(sys.argv[1])"]).arg(source.join("synthetic.docx")).status().unwrap().success());
        }
        let a=settings.sync_pairs[0].id;let b=settings.sync_pairs[1].id;
        let originals:Vec<_>=settings.sync_pairs.iter().map(|p|fs::read(p.source_folder.join("synthetic.docx")).unwrap()).collect();
        let path=config.join("settings.json");save_settings(&path,&settings).unwrap();let controller=RunController::new(path.clone());
        let mut after=detection::save_processing_config(&controller,&sidecar,a,rule("CanaryAlpha","de_core_news_lg")).await.unwrap();
        assert_eq!(after.sync_pairs[1],settings.sync_pairs[1]);
        after=detection::save_processing_config(&controller,&sidecar,b,rule("CanaryBeta","de_core_news_lg")).await.unwrap();
        for id in [a,b] {
            let run=controller.prepare(id,None,vec![]).unwrap();
            let summary=controller.execute(run,Ok(sidecar.clone()), |_|{}).await;
            assert_eq!(summary.outcome,RunOutcome::Completed,"{:?}",summary.error);
        }
        let output_a=fs::read_to_string(after.sync_pairs[0].target_folder.join("doc-0001.md")).unwrap();
        let output_b=fs::read_to_string(after.sync_pairs[1].target_folder.join("doc-0001.md")).unwrap();
        assert!(!output_a.contains("CanaryAlpha"));assert!(output_a.contains("CanaryBeta"));
        assert!(output_b.contains("CanaryAlpha"));assert!(!output_b.contains("CanaryBeta"));
        let b_saved=after.sync_pairs[1].clone();
        let changed=detection::save_processing_config(&controller,&sidecar,a,rule("CanaryBeta","de_core_news_lg")).await.unwrap();
        assert_ne!(changed.sync_pairs[0].processing_revision,after.sync_pairs[0].processing_revision);
        assert_eq!(changed.sync_pairs[1],b_saved);
        assert_eq!(detection::scan_configured(&controller,&sidecar,a).await.unwrap().files[0].state,DocumentState::Stale);
        assert_eq!(detection::scan_configured(&controller,&sidecar,b).await.unwrap().files[0].state,DocumentState::Current);
        assert_eq!(fs::read_to_string(b_saved.target_folder.join("doc-0001.md")).unwrap(),output_b);
        let bytes=fs::read(&path).unwrap();
        detection::save_processing_config(&controller,&sidecar,a,changed.sync_pairs[0].config.clone()).await.unwrap();
        assert_eq!(fs::read(&path).unwrap(),bytes);
        for invalid in [rule("[","de_core_news_lg"),rule("CanaryAlpha","missing_model")] {
            assert!(detection::save_processing_config(&controller,&sidecar,a,invalid).await.is_err());
            assert_eq!(fs::read(&path).unwrap(),bytes);
        }
        let selected=detection::save_processing_config(&controller,&sidecar,b,rule("CanaryAlpha","de_core_news_sm")).await.unwrap();
        assert_eq!(selected.sync_pairs[0],changed.sync_pairs[0]);
        assert_eq!(selected.sync_pairs[1].processing_fingerprint.as_ref().unwrap().model_name,"de_core_news_sm");
        for (id,expected) in [(a,(12,22)),(b,(0,11))] {
            let pair=selected.sync_pairs.iter().find(|p|p.id==id).unwrap();
            let preview=detection::preview_rules(&controller,&sidecar,id,pair.config.clone(),"CanaryAlpha CanaryBeta".into()).await.unwrap();
            assert_eq!(preview.len(),1);assert_eq!((preview[0].start,preview[0].end),expected);
        }
        let bytes=fs::read(&path).unwrap();
        let timeout=detection::preview_rules(&controller,&sidecar,a,rule("(?:(a|aa)+)+$","de_core_news_lg"),format!("{}!","a".repeat(200))).await.unwrap_err();
        assert_eq!(timeout.code,"engine_timeout");assert_eq!(fs::read(&path).unwrap(),bytes);
        let restored=detection::preview_rules(&controller,&sidecar,a,selected.sync_pairs[0].config.clone(),"CanaryAlpha CanaryBeta".into()).await.unwrap();
        assert_eq!((restored[0].start,restored[0].end),(12,22));
        for (index,pair) in settings.sync_pairs.iter().enumerate() {assert_eq!(fs::read(pair.source_folder.join("synthetic.docx")).unwrap(),originals[index]);}
        assert_eq!(load_settings(&path).unwrap(),selected);
        let audit=fs::read_to_string(config.join("audit-log.jsonl")).unwrap();
        assert!(!audit.contains("Canary"));
        sidecar.shutdown().await;
    });
}

#[test]
#[ignore = "requires reviewed Python and bundled models"]
fn real_producer_own_version_rotates_only_used_pair_once() {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config"); fs::create_dir(&config).unwrap();
        let mut settings = Settings::default();
        for name in ["A", "B"] {
            let source = root.path().join(format!("{name}-in"));
            let target = root.path().join(format!("{name}-out"));
            fs::create_dir(&source).unwrap(); fs::create_dir(&target).unwrap();
            settings.add(name, &source, &target).unwrap();
            settings.sync_pairs.last_mut().unwrap().config.model = "de_core_news_sm".into();
        }
        let path = config.join("settings.json"); save_settings(&path, &settings).unwrap();
        let controller = RunController::new(path.clone());
        let a = settings.sync_pairs[0].id;
        let sidecar = |version: &str| Sidecar::new(
            std::env::var_os("REDACTIO_TEST_PYTHON").unwrap().into(),
            vec!["-c".into(), format!("import redactio_sidecar.engine as engine; engine.ENGINE_VERSION = {version:?}; from redactio_sidecar.ipc import main; main()").into()],
            std::env::var_os("REDACTIO_MODEL_DIR").unwrap().into(),
        );
        let old = sidecar("redactio-sidecar 0.1.0");
        let before = detection::refresh_processing_config(&controller, &old, a).await.unwrap();
        old.shutdown().await;
        let new = sidecar("redactio-sidecar 0.2.0");
        let after = detection::refresh_processing_config(&controller, &new, a).await.unwrap();
        assert_ne!(before.sync_pairs[0].processing_revision, after.sync_pairs[0].processing_revision);
        assert_eq!(before.sync_pairs[1], after.sync_pairs[1]);
        let fingerprint = after.sync_pairs[0].processing_fingerprint.as_ref().unwrap();
        assert!(fingerprint.engine_version.contains("redactio-sidecar 0.2.0"));
        assert!(fingerprint.engine_version.contains("presidio-analyzer"));
        assert!(fingerprint.engine_version.contains("spacy"));
        assert!(!fingerprint.engine_version.contains(root.path().to_str().unwrap()));
        assert_eq!(before.sync_pairs[0].processing_fingerprint.as_ref().unwrap().model_version, fingerprint.model_version);
        let bytes = fs::read(&path).unwrap();
        assert_eq!(detection::refresh_processing_config(&controller, &new, a).await.unwrap(), after);
        assert_eq!(fs::read(path).unwrap(), bytes);
        new.shutdown().await;
    });
}
