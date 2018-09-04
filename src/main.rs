// использование этой директивы позволяет погасить консольное окно.
// но! после это, например, консольный логгер будет паниковать при создании.
#![windows_subsystem="windows"]

#[macro_use] extern crate log;
extern crate simplelog;
extern crate config;
extern crate chrono;
extern crate fs_extra;
extern crate regex;
extern crate encoding_rs;
extern crate sciter;

use std::{fs, io, panic};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::ffi::{OsStr};
use simplelog::*;
use regex::Regex;
use encoding_rs::WINDOWS_1251;
use chrono::prelude::*;

// rustsym panics on this:
// use sciter::{Element, dom::event::*, dom::HELEMENT, value::Value};
// so i temporary expand it
use sciter::Element;
use sciter::dom::event::*;
use sciter::dom::HELEMENT;
use sciter::value::Value;

// traits
use std::str::FromStr;
use std::io::{Read, Write, BufRead, Seek};

const PKG_NAME: &'static str = env!("CARGO_PKG_NAME");

//const DEFAULT_TERM_LOG_LEVEL: LevelFilter = LevelFilter::Debug;
const DEFAULT_FILE_LOG_LEVEL: LevelFilter = LevelFilter::Debug;

struct EventHandler<'a> {
    settings: &'a config::Config
}

impl<'a> sciter::EventHandler for EventHandler<'a> {

    fn on_event(&mut self
        , root: HELEMENT, source: HELEMENT, _target: HELEMENT, code: BEHAVIOR_EVENTS
        , phase: PHASE_MASK, _reason: EventReason) -> bool {

        if phase != PHASE_MASK::BUBBLING {
                return false;
        }

    if code == BEHAVIOR_EVENTS::BUTTON_CLICK {

        // `root` points to attached element, usually it is an `<html>`.

        let root = Element::from(root).root();

        let message = root.find_first("#message").unwrap().expect("div#message not found");
        let source = Element::from(source);

        println!("our root is {:?}, message is {:?} and source is {:?}", root, message, source);

        if let Some(id) = source.get_attribute("id") {
            println!("id = {:?}", id);
            if id == "do_work" {
                // just send a simple event
                return do_update_generation(&mut self.settings, &source, message.as_ptr()).is_ok();
            }

            if id == "do_test" {
                let data = Value::from("Rusty param");

                source
                    .fire_event(BEHAVIOR_EVENTS::CHANGE, None, Some(message.as_ptr()), false, Some(data))
                    .expect("Failed to fire event");
            }
        }
    }

    false
    }
}


fn do_update_generation(settings: &config::Config, html: &Element, msg: HELEMENT) -> std::io::Result<()> {

    let println_ui_msg = |text: &str| {
        html.fire_event(BEHAVIOR_EVENTS::CHANGE, None, Some(msg), false
                , Some(Value::from(format!(" -->> {}\r\n", text))))
            .expect("Failed to fire event");
    };

    // херивознает как лучше сделать.
    let exclude_file = PathBuf::from("exclude.txt");

    let new_files_root_dir =
            PathBuf::from(settings.get::<String>("main.new_files_root_dir").unwrap().as_str());
    let update_location_root =
            PathBuf::from(settings.get::<String>("main.update_location_root").unwrap().as_str());

    info!("Проверка существования директори источника файлов {:?}", new_files_root_dir);

    if !new_files_root_dir.exists() {
        println_ui_msg(
            format!("Директория источника файлов {:?} не существует! Работа прекращена"
                    , new_files_root_dir)
                .as_str());

        return Ok(());
    }

    let sample_root_dir =
            new_files_root_dir
                .join(PathBuf::from(settings.get::<String>("main.sample_rel_dir").unwrap().as_str()));

    info!("Проверка существования директории с образцами {:?}", sample_root_dir);

    if !sample_root_dir.exists() {
        println_ui_msg(
            format!("Директория образцов {:?} не существует! Работа прекращена", sample_root_dir)
                .as_str());
        return Ok(());
    }

    let exclude_list =
        if exclude_file.exists() && exclude_file.is_file() {
            info!("Нашли файл исключений {:?}. Прочтём и запомним, что надо исключить."
                , exclude_file);

            io::BufReader::new(&fs::File::open(exclude_file).unwrap())
                .lines()
                .map(|l|
                        Box::new(PathBuf::from(l.unwrap().trim_right()))
                        //Box::new(PathBuf::from(String::from("test")))
                        //Box::new(Path::new("test"))
                )
                .collect()
        } else { Vec::new() };

    /* Теперь нужно пройтись по файлам директории,
        собрать там файлы, которые не запрещены
        из остальных оставить class-файлы и sql-файлы, собрав их в два списка
    */

    info!("Читаем директорию {:?} в поисках нужных нам файлов.", new_files_root_dir);

    let mut sql_list : Vec<PathBuf> = Vec::new();
    let mut class_list : Vec<PathBuf> = Vec::new();

    for item in fs::read_dir(new_files_root_dir)
            .unwrap()
            .map(|i| i.unwrap().path())
            .filter(
                |p|
                    !exclude_list
                        .iter()
                        .any(|x| **x == p.file_name().unwrap())
                    && p.is_file()) {

        match &item.extension().and_then(OsStr::to_str) {
            Some("sql") => { sql_list.push(item); }
            ,Some("class") => { class_list.push(item); }
            ,_           => ()
        }
    }

    if sql_list.iter().count() < 1 && class_list.iter().count() < 1 {
        info!("Файлов для работы не найдено. Успешно завершаемся.");
        println_ui_msg("Файлов для работы не найдено. Успешно завершаемся.");

        return Ok(());
    }

    // Так, кажись, работёнка.

    /* вычислить директорию сегодняшних обновлений.
        Она должна быть вида <update_location_root>\update_<year>\update_<year_mon_day>
        Если не существует - создать*/
    let local_dt = chrono::Local::now();

    let mut cur_update_dir =
        update_location_root
            .join(format!("update_{}", local_dt.format("%Y").to_string()))
            .join(format!("update_{}", local_dt.format("%Y.%m.%d")));

    /* 2. Папку сегодняшних обновлений обеспечили.
        Теперь подобрать имя прямо для текущего обновления! Вот казалось бы, ну что за развлечение.
        Но вот. Типа, хотим файл "3. Разное",  но если такой есть, то нужен файл
        "4. Разное" ну и т.д.*/

    let mut ind  = 1;

    loop {
        if !cur_update_dir.join(format!("{}. Разное", ind)).exists() { break; }
        else { ind += 1; }
    }

    // вот конечная точка, в которой мы наконец будем файлы создавать
    cur_update_dir.push(format!("{}. Разное", ind));

    fs::create_dir_all(&cur_update_dir)?;

    info!("Копирование содержимого директории с образцами {:?} в итоговую \
           директорию {:?}"
        , &sample_root_dir, &cur_update_dir);

    // чую что это можно элегантнее сделать, но не знаю кк.
    let dir_copy_opts = fs_extra::dir::CopyOptions::new();

    // оказалось, что я не могу найти метода, чтобы он скопировал по указанному
    //   пути так, чтобы войти внутрь путя и отттуда всё внутри скопировать по
    //   указанной директории назнчения.
    // Вместо этого придётся пройтись по всем элементам содержимого исходной папки
    //    и отправить это в папку назначения.

    for item in fs::read_dir(&sample_root_dir).unwrap() {
        let i = item.unwrap().path();
        if i.is_dir() {
            fs_extra::dir::copy(&i, &cur_update_dir, &dir_copy_opts).unwrap();
        } else if i.is_file() {
            fs::copy(&i, cur_update_dir.join(i.file_name().unwrap())).unwrap();
        } else {
            panic!("{:?} - и не файл и не директория. о.0.");
        }

    }

    info!("Обновления будут созданы в {:?}", &cur_update_dir);
    println_ui_msg(format!("Обновления будут созданы в {:?}", &cur_update_dir).as_str());

    // 1. *.sql файлы перемещаем в корень и в переменную, чтобы потом её в _install.sql добавить.
    // 2. *.class перемещаем в load

    let mut file_list = String::new();

    for f in &sql_list {
        info!("Перемещаем sql-файл {:?} в итоговую директорию", &f);
        println_ui_msg(format!("Перемещаем sql-файл {:?} в итоговую директорию", &f).as_str());

        fs::rename(&f, cur_update_dir.join(f.file_name().unwrap()))?;
        file_list.push_str(
            format!(
                r#"@"&PREFIX\{}"{}"#
                , f.file_name().unwrap().to_str().unwrap()
                ,"\r\n")
                .as_str());
    }

    let cur_class_dir = cur_update_dir.join("load");
    for f in &class_list {
        info!("Перемещаем class-файл {:?} в итоговую директорию", &f);
        println_ui_msg(
            format!("Перемещаем class-файл {:?} в итоговую директорию", &f).as_str());

        fs::rename(f, cur_class_dir.join(f.file_name().unwrap()))?;
    }

    /* теперь более магическая магия.
        Нужно в файле cur_update_dir/_install.sql
            1. Вписать нужный PREFIX
            2. Добавить список файлов.*/

    /* кабуто так просто в обычных языках взять и изнасиловать файл.
        Нужна прочитать, изменить и записать заново */

    let install_file = cur_update_dir.join("_install.sql");

    info!("Открываем для переписывания и дописывания {:?}", &install_file);

    let mut install_template = fs::File::open(&install_file)?;
    let mut orig_template = String::new();

    info!("Вычитываем содержимое {:?} для дальнейшего изменения", &install_file);

    // обработаем ситуацию, когда пытались читать файл, а там оказался не utf-8
    // наивно предполагаем, что если не получилось чёта прочитать, то это 1251 =)
    match install_template.read_to_string(&mut orig_template) {
        Ok(_) => {
            info!("read_to_string seems ok!");
        }
        ,Err(ref err) if std::io::ErrorKind::InvalidData == err.kind() => {
            info!("Обнаружили ошибку чтения индекс-файла. Пробуем конвертировать");
            let mut template_1251 = Vec::new();
            // сбросим файл на начало, потому что это типа поток.
            install_template.seek(std::io::SeekFrom::Start(0)).unwrap();
            install_template.read_to_end(&mut template_1251).unwrap();

            let (cow, _encoding_used, had_errors)
                = WINDOWS_1251.decode(&template_1251);

            orig_template = String::from(cow);

            info!("Были ошибки конвертации: {:?}", had_errors);
            debug!("Наконвертировали: {:?}", &orig_template);
        }
        ,Err(err) => panic!("Неожиданная ошибка: {:?}", err)
    }

    drop(install_template);  // Close the file early

    info!("Подменяем переменную PREFIX");

    // (?m) в мочале регекспа - это multi-line режим, когда ^ матчит начало строки, а $ - конце
    //    строки. Без этого они матчат только мочало и конец файла.

    let mut new_data =
        String::from(
            Regex::new(r"(?m)^define PREFIX=.*$")
                .unwrap()
                .replace(
                    &orig_template
                    , format!(r#"define PREFIX="{}""#, cur_update_dir.to_str().unwrap())
                        .as_str()));

    new_data.push_str(&file_list);

    // в уютненьком sqlplus можно работать только с 1251, будем конвертировать

    let (finally_1251, _encoding_used, _had_errors)
        = WINDOWS_1251.encode(&new_data);

    info!("Создаём пустой {:?}", &install_file);

    let mut final_install = fs::File::create(&install_file)?;

    info!("Пишем изменённые данные в {:?}", &install_file);

    final_install.write(&Vec::from(finally_1251))?;

    info!("Ну и наконец дописываем список файлов.");

    drop(final_install);

    info!("Всё наконец!");
    println_ui_msg("Всё наконец!");

    Ok(())
}

fn main() -> std::io::Result<()> {
    // в оконном приложении паниковать дефолтно трудно.
    // паниковать в логи трудно, например, если логи ещё не сконфигурирован. Поэтому паниковать
    // будем в отдельный файл panic.log.

    let orig_panic = panic::take_hook();

    panic::set_hook(Box::new(move |e: &panic::PanicInfo| {
        // паники изнутри этого метода до нас никогда не долетят.
        let mut panic_file = OpenOptions::new()
            .append(true)
            .create(true)
            .open("panic.log")
            .unwrap();

        panic_file.write_fmt(format_args!("{} {:?}\r\n", Local::now(), &e)).unwrap();

        drop(panic_file);

        orig_panic(e);
    }));


    // чтение конфига из settings.toml
    let mut settings = config::Config::default();
    settings.merge(config::File::with_name("settings")).unwrap();

    CombinedLogger::init(
        vec![
            // искал какой-нибудь способ в настройки логгера передать числовое значение - не нашёл.
            // реализовывать свитч самому здесь - лень ваще.
            // Отключил консольный вывод, чтобы под вендой красиво работало.
            // TermLogger::new(
            //             LevelFilter::from_str(
            //                 settings.get::<String>("main.term_log_level").unwrap().as_str())
            //             .unwrap_or(DEFAULT_TERM_LOG_LEVEL)
            //         , Config::default())
            //     .expect("Failed to create TermLogger")
            // ,
            WriteLogger::new(
                //LevelFilter::from_str(sets.get("main.file_log_level").unwrap())
                LevelFilter::from_str(settings.get::<String>("main.file_log_level").unwrap().as_str())
                    .unwrap_or(DEFAULT_FILE_LOG_LEVEL)
                ,Config::default()
                ,OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(Path::new(OsStr::new(&format!("{}.log", PKG_NAME))))
                    .unwrap())]).expect("Failed to configure logging.");

    info!("Started. Version = {}", env!("CARGO_PKG_VERSION"));

    // Step 1: Include the 'minimal.html' file to a byte array.
    // Магическая штука - включает байты из кода в переменную в момент компиляции.
    // Считай что я сразу в неё влюбился )
    let html = include_bytes!("face.htm");

    // Step 2: Create a new main sciter window of type sciter::Window
    let mut frame = sciter::Window::new();

    frame.event_handler(EventHandler {settings: &settings});

    // Step 3: Load HTML byte array from memory to sciter::Window.
    // Второй параметр опциональный, помогает при отладке.
    frame.load_html(html, Some("example://face.htm"));
    frame.set_title(
        format!("{}: Погоматель упаковки обновлений ({})", PKG_NAME,
            env!("CARGO_PKG_VERSION")).as_str());

    // Step 4: Show window and run the main app message loop until window been closed.
    frame.run_app();

    return Ok(());
}
