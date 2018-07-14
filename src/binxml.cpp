
binxml_node* binxml_template::get_child(binxml_node *node, std::size_t pos)
{
    auto itr = node->begin();
    std::advance(itr, pos);
    return &itr->second;
}

std::size_t binxml_template::count_substitutions() const
{
    return subs.size();
}

binxml_node* binxml_template::get_substitution(uint16_t index, binxml_node *root) const
{
    binxml_node *node = root;

    auto itr = subs.find(index);
    if(itr != subs.end())
        for(auto p:itr->second)
            node = get_child(node, p);

    else node = nullptr;

    return node;
}

void binxml_template::add_substitution(uint16_t index, const path &p)
{
    subs.emplace(index, p);
}

binxml_parser::binxml_parser()
        : stop(true),
          current_chunk(nullptr),
          root(nullptr)
{

}


void binxml_parser::on_end_of_stream()
{
    stack.pop_back();
    stop = true;
}

void binxml_parser::on_open_start_element(bool more_bits)
{
    skip(2); //unknown

    uint32_t size;
    read(size);

    std::string name = read_string();
    if(more_bits) skip(4);

    auto *top = stack.back();
    stack_path.push_back(top->size());

    auto &child = top->add_child(name, binxml_node());
    stack.push_back(&child);
}

void binxml_parser::on_close_start_element()
{

}

void binxml_parser::on_close_empty_element()
{
    stack_path.pop_back();
    stack.pop_back();
}

void binxml_parser::on_close_element()
{
    stack_path.pop_back();
    stack.pop_back();
}

void binxml_parser::on_value(bool more_bits)
{
    uint8_t type;
    uint16_t size;
    read(type);
    read(size);

    value val;
    val.read(get_stream(), static_cast<value_type>(type), size);

    stack.back()->put_value(val);
    std::string subpath = stack[stack.size() - 2]->back().first;

    if(subpath == "<xmlattr>")
    {
        stack_path.pop_back();
        stack_path.pop_back();
        stack.pop_back();
    }
}

void binxml_parser::on_attribute(bool more_bits)
{
    std::string name = read_string();
    std::string path = "<xmlattr>." + name;

    auto *top = stack.back();
    auto &attr = top->add_child(path, binxml_node());
    stack.push_back(&attr);

    auto itr = top->find("<xmlattr>");
    stack_path.push_back(std::distance(top->to_iterator(itr), top->begin()));
    stack_path.push_back(itr->second.size() - 1);
}

void binxml_parser::on_cdata_section()
{

}

void binxml_parser::on_entity_reference()
{

}

void binxml_parser::on_processing_instruction_target()
{

}

void binxml_parser::on_processing_instruction_data()
{

}

void binxml_parser::on_template_instance()
{
    skip(1);

    uint32_t template_id, template_offset, next_offset;
    read(template_id);
    read(template_offset);
    read(next_offset);

    bool new_template = false;
    if(!current_chunk->has_template(template_id))
    {
        new_template = true;

        read(template_id);
        skip(16);

        binxml_template node;
        binxml_parser parser;
        parser.parse(get_stream(), *current_chunk, node, true);

        current_chunk->add_template(template_id, node);
    }

    auto &tmpl = current_chunk->get_template(template_id);
    *root = tmpl;

    uint32_t size;
    if(new_template) read(size);
    else size = tmpl.count_substitutions();

    value_spec vs_arr[size];
    for(std::size_t i = 0; i < size; ++i)
    {
        read(vs_arr[i].size);
        read(vs_arr[i].type);
        if(vs_arr[i].type == value_type::wstring_type) vs_arr[i].size /= 2;
    }

    for(std::size_t i = 0; i < size; ++i)
    {
        auto *substitution = tmpl.get_substitution(i, root);
        if(vs_arr[i].type == value_type::bxml_type)
        {
            auto &stream = get_stream();
            while(stream.peek() != 0x0f) stream.get();

            binxml_node node;
            binxml_parser bxml;
            bxml.parse(get_stream(), *current_chunk, node);
            if(substitution) std::copy(node.begin(), node.end(), std::back_inserter(*substitution));

        }
        else
        {
            value val;
            val.read(get_stream(), vs_arr[i]);
            if(substitution) substitution->put_value(val);
        }
    }

/*
boost::property_tree::xml_writer_settings<char> settings('\t', 1);
boost::property_tree::write_xml(std::cout, *root, settings);
std::cout << std::endl;
*/
    stop = true;
}

void binxml_parser::on_normal_substitution()
{
    uint16_t index;
    uint8_t type;

    read(index);
    read(type);

    if(is_template_definition) static_cast<binxml_template*>(root)->add_substitution(index, stack_path);

    std::string subpath = stack[stack.size() - 2]->back().first;
    if(subpath == "<xmlattr>")
    {
        stack_path.pop_back();
        stack_path.pop_back();
        stack.pop_back();
    }
}

void binxml_parser::on_conditional_substitution()
{
    uint16_t index;
    uint8_t type;

    read(index);
    read(type);

    if(is_template_definition) static_cast<binxml_template*>(root)->add_substitution(index, stack_path);

    std::string subpath = stack[stack.size() - 2]->back().first;
    if(subpath == "<xmlattr>")
    {
        stack_path.pop_back();
        stack_path.pop_back();
        stack.pop_back();
    }
}

void binxml_parser::on_start_of_stream()
{
    skip(3);
    stack.push_back(root);
}

std::string binxml_parser::read_string()
{
    std::string retval;
    uint32_t string_offset;
    read(string_offset);

    if(current_chunk->has_string(string_offset)) retval = current_chunk->get_string(string_offset);
    else
    {
        uint32_t next_offset;
        read(next_offset);

        uint16_t hash, string_length;
        read(hash);
        read(string_length);

        to_utf8(retval, string_length);

        skip(2);

        current_chunk->add_string(string_offset, retval);
    }
    return retval;
}

void binxml_parser::to_utf8(std::string &str, std::size_t length) const
{
    uint16_t buf[length];
    for(std::size_t i = 0; i < length; ++i) read(buf[i]);
    str = boost::locale::conv::utf_to_utf<char>(buf, buf + length);
}

std::string binxml_parser::get_current_path() const
{
    std::string path;
    for(auto &x:stack_path)
    {
        if(!path.empty()) path += '.';
        path += x;
    }
    return path;
}

} // namespace pevtx